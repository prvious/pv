use std::io::{self, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener, TcpStream as StdTcpStream};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, ensure};
use camino::Utf8Path;
use daemon::RunningDaemon;
use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::rdata::A;
use hickory_proto::rr::{Name, RData, Record, RecordType};
use hickory_proto::serialize::binary::BinEncodable;
use platform::{
    PlatformError, PrivilegedHelperClient, ResolverConfig, ResolverFileState,
    SYSTEM_RESOLVER_TEST_PATH,
};
use serde_json::{Value, json};
use state::{Database, PortOwner, PvPaths, fs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::{
    HelperFixture, checked, checksum, require_units_absent, root, root_text, service_property,
    text, with_helper_fixture,
};

const CONTROL: &str = "/run/pv-dns-system-test";
const CLEANUP_UNIT: &str = "pv-dns-system-cleanup";
const FOREIGN_LINK: &str = "pv-dns-foreign";
const DNSSEC_FILE: &str = "/etc/systemd/resolved.conf.d/99-pv-dns-system-test.conf";
const RESOLVED_RUNTIME_MASK: &str = "/run/systemd/system/systemd-resolved.service";
const DNS_PATHS: [&str; 6] = [
    SYSTEM_RESOLVER_TEST_PATH,
    "/etc/systemd/system/pv-dns.service.candidate",
    "/etc/systemd/system/pv-dns.service.d",
    "/etc/systemd/system/systemd-resolved.service.wants/pv-dns.service",
    "/run/systemd/system/systemd-resolved.service.wants/pv-dns.service",
    DNSSEC_FILE,
];

// Both callers own only paths and link names proven absent before the timer is armed.
// The timer first stops the test cgroup; both paths stop the helper so queued work
// cannot recreate DNS state while administrative fixture cleanup is in progress.
const CLEANUP_SCRIPT: &str = r#"set -eu
stop_units() {
    /usr/bin/systemctl --no-block stop "$@" || :
    /usr/bin/systemctl kill --kill-who=all --signal=KILL "$@" || :
    stopped=false
    for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
        remaining=$(/usr/bin/systemctl list-units --all --no-legend --no-pager --plain \
            --state=active,activating,deactivating,reloading "$@") || return 1
        if [ -z "$remaining" ]; then stopped=true; break; fi
        sleep 0.5
    done
    [ "$stopped" = true ]
}
if [ "${1-}" = orphan ]; then
    stop_units pv-helper-system-test.service
fi
stop_units pv-helper.socket pv-helper.service
stop_units pv-dns.service
failed=$(/usr/bin/systemctl list-units --all --no-legend --no-pager --plain \
    --state=failed pv-dns.service)
if [ -n "$failed" ]; then
    /usr/bin/systemctl reset-failed pv-dns.service
fi
if [ -f /run/pv-dns-system-test/created-resolved-mask ]; then
    /usr/bin/systemctl unmask --runtime systemd-resolved.service
fi
for link in pv0 pv-dns-foreign; do
    if /usr/sbin/ip link show dev "$link" >/dev/null 2>&1; then
        /usr/sbin/ip link delete dev "$link"
    fi
done
rm -f -- /etc/systemd/system/pv-dns.service /etc/systemd/system/pv-dns.service.candidate \
    /etc/systemd/system/systemd-resolved.service.wants/pv-dns.service \
    /run/systemd/system/systemd-resolved.service.wants/pv-dns.service \
    /etc/systemd/resolved.conf.d/99-pv-dns-system-test.conf
rm -rf -- /etc/systemd/system/pv-dns.service.d
if [ -f /run/pv-dns-system-test/created-resolved-directory ] && [ -d /etc/systemd/resolved.conf.d ]; then
    rmdir -- /etc/systemd/resolved.conf.d
fi
/usr/bin/systemctl daemon-reload
/usr/bin/systemctl restart systemd-resolved.service
# Retain the root-owned control directory and timer journal for failure diagnostics.
"#;

#[test]
#[ignore = "requires PV_LINUX_SYSTEM_TESTS=1, the built helper and a disposable bounded systemd service with resolved and dummy links"]
fn systemd_dns_routes_test_reapplies_after_restart_changes_port_and_refuses_conflicts() -> Result<()>
{
    with_helper_fixture(|fixture, helper| with_dns_fixture(fixture, helper, DnsFixture::exercise))
}

#[test]
#[ignore = "requires PV_LINUX_SYSTEM_TESTS=1, the built helper and a disposable bounded systemd service with resolved"]
fn systemd_dns_inspection_does_not_activate_stopped_resolved() -> Result<()> {
    with_helper_fixture(|fixture, helper| {
        with_dns_fixture(fixture, helper, DnsFixture::inspect_stopped)
    })
}

#[test]
#[ignore = "requires PV_LINUX_SYSTEM_TESTS=1, the built helper and a disposable bounded systemd service with resolved and dummy links"]
fn systemd_dns_repairs_retained_owned_endpoint_defects() -> Result<()> {
    with_helper_fixture(|fixture, helper| {
        with_dns_fixture(fixture, helper, DnsFixture::repair_retained)
    })
}

#[test]
#[ignore = "requires PV_LINUX_SYSTEM_TESTS=1, the built helper and a disposable bounded systemd service with resolved and dummy links"]
fn systemd_dns_preserves_foreign_interface_and_endpoint_state() -> Result<()> {
    with_helper_fixture(|fixture, helper| {
        with_dns_fixture(fixture, helper, DnsFixture::preserve_foreign)
    })
}

#[test]
#[ignore = "requires PV_LINUX_SYSTEM_TESTS=1, the built helper and a disposable bounded systemd service with resolved and dummy links"]
fn systemd_dns_uses_both_transports_and_isolates_eligible_scopes() -> Result<()> {
    with_helper_fixture(|fixture, helper| with_dns_fixture(fixture, helper, DnsFixture::transport))
}

fn with_dns_fixture(
    helper_fixture: &mut HelperFixture,
    helper: &Utf8Path,
    exercise: fn(&mut DnsFixture, &mut HelperFixture, &Utf8Path) -> Result<()>,
) -> Result<()> {
    preflight()?;
    let paths = PvPaths::for_home(helper_fixture.scratch.path().join("dns-home"));
    let mut fixture = DnsFixture {
        paths,
        runtime: Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?,
        daemon: None,
        control_created: false,
        timer_attempted: false,
        cleaned: false,
    };
    let result = exercise(&mut fixture, helper_fixture, helper);
    let cleanup = fixture.cleanup();
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("cleanup also failed: {cleanup:#}")))
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn preflight() -> Result<()> {
    for path in DNS_PATHS
        .into_iter()
        .chain([CONTROL, RESOLVED_RUNTIME_MASK])
    {
        ensure!(
            !fs::path_entry_exists(Utf8Path::new(path))?,
            "refusing existing DNS path {path}"
        );
    }
    let links = links()?;
    ensure!(
        !links
            .iter()
            .any(|link| matches!(link["ifname"].as_str(), Some("pv0" | FOREIGN_LINK))),
        "refusing existing fixture link"
    );
    assert_eq!(
        service_property("systemd-resolved.service", "ActiveState")?,
        "active"
    );
    ensure!(
        matches!(
            platform::inspect_resolver_file(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH), None),
            ResolverFileState::Missing { .. }
        ),
        "DNS routing must initially be missing without conflicts"
    );
    let nameservers: Vec<_> = fs::read_to_string(Utf8Path::new("/etc/resolv.conf"))?
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("nameserver"))
                .then(|| words.next().map(str::to_owned))
                .flatten()
        })
        .collect();
    ensure!(
        nameservers == ["127.0.0.53"],
        "the disposable machine must use only resolved's stub nameserver"
    );
    let os_release = fs::read_to_string(Utf8Path::new("/etc/os-release"))?;
    if os_release
        .lines()
        .any(|line| matches!(line, "ID=fedora" | "ID=\"fedora\""))
    {
        ensure!(
            service_property("NetworkManager.service", "ActiveState")? == "active",
            "start NetworkManager before running the Fedora DNS test"
        );
    }
    require_units_absent(&[
        "pv-dns.service",
        "pv-dns-system-cleanup.timer",
        "pv-dns-system-cleanup.service",
    ])?;
    Ok(())
}

struct DnsFixture {
    paths: PvPaths,
    runtime: Runtime,
    daemon: Option<RunningDaemon>,
    control_created: bool,
    timer_attempted: bool,
    cleaned: bool,
}

impl DnsFixture {
    fn transport(&mut self, helper_fixture: &mut HelperFixture, helper: &Utf8Path) -> Result<()> {
        self.arm_cleanup(helper_fixture)?;
        helper_fixture.arm_cleanup()?;
        helper_fixture.install(helper, &checksum(helper)?)?;
        helper_fixture.capture_process()?;
        let port = pv_fake::seed_dns_port(&self.paths)?;
        let foreign_port = pv_fake::port()?;
        let canonical_port = pv_fake::port()?;
        let _canonical_listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, canonical_port))?;
        canonical_observation("before", canonical_port)?;
        let suffix = helper_fixture
            .scratch
            .path()
            .file_name()
            .context("fixture directory name missing")?
            .trim_start_matches('.')
            .to_ascii_lowercase();
        self.start_daemon()?;
        PrivilegedHelperClient.apply_dns(&ResolverConfig::new(port))?;
        require_current(port)?;
        require_owned_endpoint()?;
        self.runtime.block_on(require_production_transports(
            port,
            &format!("wire.{suffix}.pv-dns-system.test"),
        ))?;
        canonical_observation("during", canonical_port)?;
        self.stop_daemon()?;
        let addresses: Vec<Value> = serde_json::from_slice(
            &checked("/usr/sbin/ip", &["-json", "address", "show"])?.stdout,
        )?;
        for observed in addresses {
            ensure!(
                !observed["addr_info"]
                    .as_array()
                    .context("native address inventory missing")?
                    .iter()
                    .any(|address| address["local"] == "192.0.2.53"),
                "refusing existing foreign-responder address"
            );
        }
        root(&[
            "/usr/sbin/ip",
            "link",
            "add",
            "name",
            FOREIGN_LINK,
            "type",
            "dummy",
        ])?;
        root(&[
            "/usr/sbin/ip",
            "address",
            "add",
            "192.0.2.53/32",
            "dev",
            FOREIGN_LINK,
            "scope",
            "global",
        ])?;
        root(&["/usr/sbin/ip", "link", "set", "dev", FOREIGN_LINK, "up"])?;
        let trace = self.runtime.block_on(controlled_dns_probes(
            port,
            foreign_port,
            format!("fallback.{suffix}.pv-dns-system.test"),
            format!("unrelated.{suffix}.pv-foreign.example"),
        ))?;
        writeln!(io::stdout().lock(), "DNS transport trace: {trace:?}")?;
        PrivilegedHelperClient.remove_dns()?;
        require_owned_resources_removed()?;
        canonical_observation("after PV removal", canonical_port)?;
        root(&["/usr/bin/resolvectl", "revert", FOREIGN_LINK])?;
        root(&["/usr/sbin/ip", "link", "delete", "dev", FOREIGN_LINK])?;
        require_missing()?;
        Ok(())
    }

    fn repair_retained(
        &mut self,
        helper_fixture: &mut HelperFixture,
        helper: &Utf8Path,
    ) -> Result<()> {
        self.arm_cleanup(helper_fixture)?;
        helper_fixture.arm_cleanup()?;
        helper_fixture.install(helper, &checksum(helper)?)?;
        helper_fixture.capture_process()?;
        let port = pv_fake::seed_dns_port(&self.paths)?;
        self.start_daemon()?;
        let config = ResolverConfig::new(port);
        PrivilegedHelperClient.apply_dns(&config)?;
        require_current(port)?;
        let suffix = helper_fixture
            .scratch
            .path()
            .file_name()
            .context("fixture directory name missing")?
            .trim_start_matches('.');
        for (defect, failed) in [
            ("scope", false),
            ("lifetime", true),
            ("missing-route", false),
            ("route-source", true),
        ] {
            root(&["/usr/bin/systemctl", "stop", "pv-dns.service"])?;
            assert!(link("pv0")?.is_none());
            root(&["/usr/bin/systemctl", "reset-failed", "pv-dns.service"])?;
            if failed {
                root(&["/usr/bin/install", "-d", "-m", "0755", DNS_PATHS[2]])?;
                helper_fixture.root_file(
                    "failed-dns.conf",
                    "[Service]\nExecStartPre=/usr/bin/false\n",
                    &format!("{}/fixture.conf", DNS_PATHS[2]),
                    "0644",
                )?;
                root(&["/usr/bin/systemctl", "daemon-reload"])?;
                ensure!(root(&["/usr/bin/systemctl", "start", "pv-dns.service"]).is_err());
                assert_eq!(service_property("pv-dns.service", "ActiveState")?, "failed");
                root(&[
                    "/usr/bin/rm",
                    "--",
                    &format!("{}/fixture.conf", DNS_PATHS[2]),
                ])?;
                root(&["/usr/bin/rmdir", "--", DNS_PATHS[2]])?;
                root(&["/usr/bin/systemctl", "daemon-reload"])?;
                assert_eq!(service_property("pv-dns.service", "DropInPaths")?, "");
            }
            let options: &[&str] = match defect {
                "scope" => &["scope", "host"],
                "lifetime" => &[
                    "scope",
                    "global",
                    "valid_lft",
                    "600",
                    "preferred_lft",
                    "600",
                ],
                _ => &["scope", "global"],
            };
            create_owned_endpoint(options)?;
            if defect == "missing-route" {
                root(&[
                    "/usr/sbin/ip",
                    "-4",
                    "route",
                    "del",
                    "local",
                    "127.0.0.2/32",
                    "table",
                    "local",
                    "dev",
                    "pv0",
                ])?;
            } else if defect == "route-source" {
                root(&[
                    "/usr/sbin/ip",
                    "-4",
                    "route",
                    "replace",
                    "local",
                    "127.0.0.2/32",
                    "table",
                    "local",
                    "dev",
                    "pv0",
                    "proto",
                    "kernel",
                    "scope",
                    "host",
                    "src",
                    "127.0.0.1",
                ])?;
            }
            let retained = link("pv0")?.context("retained owned link missing")?;
            ensure!(retained["ifalias"] == "Managed by PV");
            let addresses: Vec<Value> = serde_json::from_slice(
                &checked("/usr/sbin/ip", &["-json", "address", "show", "dev", "pv0"])?.stdout,
            )?;
            let address = addresses
                .first()
                .and_then(|link| link["addr_info"].as_array())
                .and_then(|addresses| {
                    addresses
                        .iter()
                        .find(|address| address["local"] == "127.0.0.2")
                })
                .context("retained endpoint address missing")?;
            assert_eq!(address["prefixlen"], 32);
            assert_eq!(
                address["scope"],
                if defect == "scope" { "host" } else { "global" }
            );
            for field in ["valid_life_time", "preferred_life_time"] {
                let lifetime = address[field]
                    .as_u64()
                    .context("address lifetime missing")?;
                if defect == "lifetime" {
                    ensure!(
                        lifetime > 0 && lifetime <= 600,
                        "finite lifetime not seeded: {address}"
                    );
                } else {
                    assert_eq!(lifetime, u64::from(u32::MAX));
                }
            }
            let routes = endpoint_routes()?;
            if defect == "missing-route" {
                ensure!(routes.is_empty(), "local route still present: {routes:?}");
            } else {
                let route = routes.first().context("retained local route missing")?;
                ensure!(
                    route["type"] == "local"
                        && route["protocol"] == "kernel"
                        && route["scope"] == "host"
                );
                assert_eq!(
                    route["prefsrc"],
                    if defect == "route-source" {
                        "127.0.0.1"
                    } else {
                        "127.0.0.2"
                    }
                );
            }
            assert_eq!(
                service_property("pv-dns.service", "ActiveState")?,
                if failed { "failed" } else { "inactive" }
            );
            assert_eq!(
                link("pv0")?.context("retained link disappeared")?["ifindex"],
                retained["ifindex"]
            );
            ensure!(matches!(
                platform::inspect_resolver_file(
                    Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH),
                    Some(&config)
                ),
                ResolverFileState::Stale { .. }
            ));
            PrivilegedHelperClient
                .apply_dns(&config)
                .with_context(|| format!("repair {defect}"))?;
            require_current(port)?;
            require_owned_endpoint()?;
            require_lookup(&format!("{defect}.{suffix}.pv-dns-system.test"))?;
        }
        PrivilegedHelperClient.remove_dns()?;
        require_owned_resources_removed()?;
        Ok(())
    }

    fn preserve_foreign(
        &mut self,
        helper_fixture: &mut HelperFixture,
        helper: &Utf8Path,
    ) -> Result<()> {
        self.arm_cleanup(helper_fixture)?;
        helper_fixture.arm_cleanup()?;
        helper_fixture.install(helper, &checksum(helper)?)?;
        helper_fixture.capture_process()?;
        let port = pv_fake::seed_dns_port(&self.paths)?;
        self.start_daemon()?;
        let config = ResolverConfig::new(port);
        for foreign_address in [true, false] {
            PrivilegedHelperClient.apply_dns(&config)?;
            require_owned_endpoint()?;
            let (add, remove): (&[&str], &[&str]) = if foreign_address {
                (
                    &["address", "add", "192.0.2.2/32", "dev", "pv0"],
                    &["address", "del", "192.0.2.2/32", "dev", "pv0"],
                )
            } else {
                (
                    &[
                        "route",
                        "add",
                        "198.51.100.0/24",
                        "dev",
                        "pv0",
                        "proto",
                        "static",
                    ],
                    &["route", "del", "198.51.100.0/24", "dev", "pv0"],
                )
            };
            let mut command = vec!["/usr/sbin/ip"];
            command.extend_from_slice(add);
            root(&command)?;
            let before = foreign_owned_fact(foreign_address)?;
            let installation = owned_installation_facts()?;
            ensure!(matches!(
                platform::inspect_resolver_file(
                    Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH),
                    Some(&config)
                ),
                ResolverFileState::Conflict { .. }
            ));
            for apply in [true, false] {
                let result = if apply {
                    PrivilegedHelperClient.apply_dns(&config)
                } else {
                    PrivilegedHelperClient.remove_dns()
                };
                ensure!(matches!(result, Err(PlatformError::SystemIntegration(_))));
                assert_eq!(foreign_owned_fact(foreign_address)?, before);
                assert_eq!(owned_installation_facts()?, installation);
                require_owned_endpoint()?;
            }
            let mut command = vec!["/usr/sbin/ip"];
            command.extend_from_slice(remove);
            root(&command)?;
            require_current(port)?;
            PrivilegedHelperClient.remove_dns()?;
            require_owned_resources_removed()?;
        }
        for external_address in [true, false] {
            PrivilegedHelperClient.apply_dns(&config)?;
            require_owned_endpoint()?;
            root(&[
                "/usr/sbin/ip",
                "link",
                "add",
                "name",
                FOREIGN_LINK,
                "type",
                "dummy",
            ])?;
            root(&[
                "/usr/sbin/ip",
                "link",
                "set",
                "dev",
                FOREIGN_LINK,
                "alias",
                "Foreign DNS fixture",
                "up",
            ])?;
            if external_address {
                root(&[
                    "/usr/sbin/ip",
                    "address",
                    "add",
                    "127.0.0.2/32",
                    "dev",
                    FOREIGN_LINK,
                    "scope",
                    "global",
                ])?;
            } else {
                root(&[
                    "/usr/sbin/ip",
                    "route",
                    "add",
                    "127.0.0.2/32",
                    "dev",
                    FOREIGN_LINK,
                    "proto",
                    "static",
                ])?;
            }
            root(&[
                "/usr/sbin/ip",
                "route",
                "add",
                "203.0.113.0/24",
                "dev",
                FOREIGN_LINK,
                "proto",
                "static",
            ])?;
            let before = foreign_external_facts(external_address)?;
            let installation = owned_installation_facts()?;
            ensure!(matches!(
                platform::inspect_resolver_file(
                    Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH),
                    Some(&config)
                ),
                ResolverFileState::Conflict { .. }
            ));
            ensure!(matches!(
                PrivilegedHelperClient.apply_dns(&config),
                Err(PlatformError::SystemIntegration(_))
            ));
            assert_eq!(foreign_external_facts(external_address)?, before);
            assert_eq!(owned_installation_facts()?, installation);
            PrivilegedHelperClient.remove_dns()?;
            require_owned_resources_removed()?;
            assert_eq!(foreign_external_facts(external_address)?, before);
            root(&["/usr/sbin/ip", "link", "delete", "dev", FOREIGN_LINK])?;
            require_missing()?;
        }
        Ok(())
    }

    fn inspect_stopped(
        &mut self,
        helper_fixture: &mut HelperFixture,
        helper: &Utf8Path,
    ) -> Result<()> {
        self.arm_cleanup(helper_fixture)?;
        helper_fixture.arm_cleanup()?;
        helper_fixture.install(helper, &checksum(helper)?)?;
        helper_fixture.capture_process()?;
        ensure!(matches!(
            PrivilegedHelperClient.inspect_dns(None)?,
            ResolverFileState::Missing { .. }
        ));
        let names: Value = serde_json::from_slice(
            &checked(
                "/usr/bin/busctl",
                &[
                    "--system",
                    "--json=short",
                    "call",
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "ListActivatableNames",
                ],
            )?
            .stdout,
        )?;
        ensure!(
            names["data"][0]
                .as_array()
                .context("activatable bus names missing")?
                .iter()
                .any(|name| name == "org.freedesktop.resolve1")
        );
        for helper_inspection in [false, true] {
            root(&["/usr/bin/systemctl", "stop", "systemd-resolved.service"])?;
            assert_eq!(
                service_property("systemd-resolved.service", "LoadState")?,
                "loaded"
            );
            require_resolved_stopped()?;
            let state = if helper_inspection {
                PrivilegedHelperClient.inspect_dns(None)?
            } else {
                platform::inspect_resolver_file(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH), None)
            };
            require_resolved_stopped()?;
            ensure!(
                matches!(state, ResolverFileState::Unreadable { .. }),
                "{state:?}"
            );
            require_owned_resources_removed()?;
        }
        Ok(())
    }

    fn exercise(&mut self, helper_fixture: &mut HelperFixture, helper: &Utf8Path) -> Result<()> {
        self.arm_cleanup(helper_fixture)?;
        helper_fixture.arm_cleanup()?;
        helper_fixture.install(helper, &checksum(helper)?)?;
        helper_fixture.capture_process()?;
        if !fs::path_entry_exists(Utf8Path::new("/etc/systemd/resolved.conf.d"))? {
            root(&[
                "/usr/bin/touch",
                &format!("{CONTROL}/created-resolved-directory"),
            ])?;
            root(&[
                "/usr/bin/install",
                "-d",
                "-m",
                "0755",
                "/etc/systemd/resolved.conf.d",
            ])?;
        }
        helper_fixture.root_file(
            "dnssec.conf",
            "[Resolve]\nDNSSEC=allow-downgrade\n",
            DNSSEC_FILE,
            "0644",
        )?;
        root(&["/usr/bin/systemctl", "restart", "systemd-resolved.service"])?;
        assert_eq!(
            text(checked(
                "/usr/bin/busctl",
                &[
                    "--system",
                    "get-property",
                    "org.freedesktop.resolve1",
                    "/org/freedesktop/resolve1",
                    "org.freedesktop.resolve1.Manager",
                    "DNSSEC"
                ]
            )?)?
            .trim(),
            r#"s "allow-downgrade""#
        );

        let first_port = pv_fake::seed_dns_port(&self.paths)?;
        let lookup_suffix = helper_fixture
            .scratch
            .path()
            .file_name()
            .context("fixture directory name missing")?
            .trim_start_matches('.');
        self.start_daemon()?;
        PrivilegedHelperClient.apply_dns(&ResolverConfig::new(first_port))?;
        require_current(first_port)?;
        require_owned_endpoint()?;
        require_lookup(&format!("first-apply.{lookup_suffix}.pv-dns-system.test"))?;
        assert_eq!(
            fs::read_link(Utf8Path::new(DNS_PATHS[3]))?,
            Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH)
        );
        assert_eq!(
            root_text(&["/usr/bin/stat", "-c", "%u:%g:%a", SYSTEM_RESOLVER_TEST_PATH])?.trim(),
            "0:0:644"
        );
        assert!(
            service_property("systemd-resolved.service", "Wants")?
                .split_whitespace()
                .any(|unit| unit == "pv-dns.service")
        );
        root(&["/usr/bin/systemctl", "stop", "systemd-resolved.service"])?;
        assert_eq!(
            service_property("pv-dns.service", "ActiveState")?,
            "inactive"
        );
        assert!(link("pv0")?.is_none());
        root(&["/usr/bin/systemctl", "start", "systemd-resolved.service"])?;
        wait_until_current(first_port)?;
        require_lookup(&format!("initial.{lookup_suffix}.pv-dns-system.test"))?;

        for runtime in [false, true] {
            root(&["/usr/bin/systemctl", "disable", "pv-dns.service"])?;
            if runtime {
                root(&[
                    "/usr/bin/systemctl",
                    "enable",
                    "--runtime",
                    "pv-dns.service",
                ])?;
            }
            ensure!(matches!(
                platform::inspect_resolver_file(
                    Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH),
                    Some(&ResolverConfig::new(first_port))
                ),
                ResolverFileState::Stale { .. }
            ));
            PrivilegedHelperClient.apply_dns(&ResolverConfig::new(first_port))?;
            require_current(first_port)?;
        }
        root(&["/usr/bin/chmod", "0664", SYSTEM_RESOLVER_TEST_PATH])?;
        require_unsafe_unit_refused(first_port)?;
        root(&["/usr/bin/chmod", "0644", SYSTEM_RESOLVER_TEST_PATH])?;
        root(&["/usr/bin/chown", "1000:1000", SYSTEM_RESOLVER_TEST_PATH])?;
        require_unsafe_unit_refused(first_port)?;
        root(&["/usr/bin/chown", "0:0", SYSTEM_RESOLVER_TEST_PATH])?;
        // Hard links must share the installed unit's filesystem.
        let hard_link = DNS_PATHS[1];
        root(&["/usr/bin/ln", SYSTEM_RESOLVER_TEST_PATH, hard_link])?;
        require_unsafe_unit_refused(first_port)?;
        root(&["/usr/bin/rm", "--", hard_link])?;
        require_current(first_port)?;

        root(&["/usr/bin/systemctl", "restart", "systemd-resolved.service"])?;
        wait_until_current(first_port)?;
        require_lookup(&format!("restarted.{lookup_suffix}.pv-dns-system.test"))?;

        self.stop_daemon()?;
        Database::open(&self.paths)?.release_port(PortOwner::Dns)?;
        let second_port = pv_fake::seed_dns_port(&self.paths)?;
        assert_ne!(second_port, first_port);
        self.start_daemon()?;
        PrivilegedHelperClient.apply_dns(&ResolverConfig::new(second_port))?;
        require_current(second_port)?;
        require_lookup(&format!("changed-port.{lookup_suffix}.pv-dns-system.test"))?;
        // Model teardown interrupted after unlink, before its final reload.
        root(&["/usr/bin/systemctl", "disable", "pv-dns.service"])?;
        root(&[
            "/usr/bin/systemctl",
            "disable",
            "--runtime",
            "pv-dns.service",
        ])?;
        root(&["/usr/bin/systemctl", "daemon-reload"])?;
        root(&["/usr/bin/systemctl", "stop", "pv-dns.service"])?;
        root(&["/usr/bin/rm", "--", SYSTEM_RESOLVER_TEST_PATH])?;
        PrivilegedHelperClient.remove_dns()?;
        PrivilegedHelperClient.remove_dns()?;
        require_missing()?;

        let foreign_unit =
            "[Service]\nType=oneshot\nExecStart=/usr/bin/true\nRemainAfterExit=yes\n";
        helper_fixture.root_file(
            "foreign-dns.service",
            foreign_unit,
            SYSTEM_RESOLVER_TEST_PATH,
            "0644",
        )?;
        require_conflict(second_port)?;
        assert_eq!(
            fs::read_to_string(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH))?,
            foreign_unit
        );
        root(&["/usr/bin/rm", "--", SYSTEM_RESOLVER_TEST_PATH])?;
        root(&["/usr/bin/systemctl", "daemon-reload"])?;

        root(&[
            "/usr/sbin/ip",
            "link",
            "add",
            "name",
            "pv0",
            "type",
            "dummy",
        ])?;
        let unmarked_link = link("pv0")?.context("unmarked pv0 missing")?;
        require_conflict(second_port)?;
        assert_eq!(link("pv0")?, Some(unmarked_link));
        root(&[
            "/usr/sbin/ip",
            "link",
            "set",
            "dev",
            "pv0",
            "alias",
            "Foreign DNS fixture",
        ])?;
        let foreign_link = link("pv0")?.context("foreign pv0 missing")?;
        require_conflict(second_port)?;
        assert_eq!(link("pv0")?, Some(foreign_link));
        root(&["/usr/sbin/ip", "link", "delete", "dev", "pv0"])?;

        root(&[
            "/usr/sbin/ip",
            "link",
            "add",
            "name",
            FOREIGN_LINK,
            "type",
            "dummy",
        ])?;
        root(&["/usr/sbin/ip", "link", "set", "dev", FOREIGN_LINK, "up"])?;
        wait_for_resolved_link(FOREIGN_LINK)?;
        for domain in ["~test", "test", "acme.test"] {
            let config = ResolverConfig::new(second_port);
            PrivilegedHelperClient.apply_dns(&config)?;
            root(&["/usr/bin/resolvectl", "domain", FOREIGN_LINK, domain])?;
            let foreign_domains = text(checked("/usr/bin/resolvectl", &["domain", FOREIGN_LINK])?)?;
            ensure!(matches!(
                platform::inspect_resolver_file(
                    Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH),
                    Some(&config)
                ),
                ResolverFileState::Conflict { .. }
            ));
            ensure!(matches!(
                PrivilegedHelperClient.apply_dns(&config),
                Err(PlatformError::SystemIntegration(_))
            ));
            if domain == "~test" {
                ensure!(root(&["/usr/bin/systemctl", "restart", "pv-dns.service"]).is_err());
                assert_eq!(service_property("pv-dns.service", "ActiveState")?, "failed");
            }
            PrivilegedHelperClient.remove_dns()?;
            require_owned_resources_removed()?;
            assert_eq!(
                text(checked("/usr/bin/resolvectl", &["domain", FOREIGN_LINK])?)?,
                foreign_domains
            );
            root(&["/usr/bin/resolvectl", "revert", FOREIGN_LINK])?;
            require_missing()?;
        }
        root(&["/usr/sbin/ip", "link", "delete", "dev", FOREIGN_LINK])?;

        PrivilegedHelperClient.apply_dns(&ResolverConfig::new(second_port))?;
        root(&["/usr/bin/systemctl", "stop", "systemd-resolved.service"])?;
        root(&[
            "/usr/bin/touch",
            &format!("{CONTROL}/created-resolved-mask"),
        ])?;
        root(&[
            "/usr/bin/systemctl",
            "mask",
            "--runtime",
            "systemd-resolved.service",
        ])?;
        require_resolved_stopped()?;
        ensure!(matches!(
            platform::inspect_resolver_file(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH), None),
            ResolverFileState::Unreadable { .. }
        ));
        create_owned_endpoint(&["scope", "global"])?;
        require_owned_endpoint()?;
        require_resolved_inactive()?;
        PrivilegedHelperClient.remove_dns()?;
        require_owned_resources_removed()?;
        require_resolved_stopped()?;
        PrivilegedHelperClient.remove_dns()?;
        require_owned_resources_removed()?;
        require_resolved_stopped()?;
        root(&[
            "/usr/sbin/ip",
            "link",
            "add",
            "name",
            "pv0",
            "type",
            "dummy",
        ])?;
        let foreign_link = link("pv0")?.context("foreign pv0 missing")?;
        ensure!(matches!(
            PrivilegedHelperClient.remove_dns(),
            Err(PlatformError::SystemIntegration(_))
        ));
        assert_eq!(link("pv0")?, Some(foreign_link));
        root(&["/usr/sbin/ip", "link", "delete", "dev", "pv0"])?;
        root(&[
            "/usr/bin/systemctl",
            "unmask",
            "--runtime",
            "systemd-resolved.service",
        ])?;
        root(&["/usr/bin/systemctl", "start", "systemd-resolved.service"])?;
        require_missing()?;
        Ok(())
    }

    fn arm_cleanup(&mut self, helper_fixture: &HelperFixture) -> Result<()> {
        root(&["/usr/bin/install", "-d", "-m", "0700", CONTROL])?;
        self.control_created = true;
        helper_fixture.root_file(
            "dns-cleanup.sh",
            CLEANUP_SCRIPT,
            &format!("{CONTROL}/cleanup.sh"),
            "0700",
        )?;
        self.timer_attempted = true;
        root(&[
            "/usr/bin/systemd-run",
            "--collect",
            "--on-active=140s",
            "--unit",
            CLEANUP_UNIT,
            "--property=RuntimeMaxSec=30s",
            "/bin/sh",
            &format!("{CONTROL}/cleanup.sh"),
            "orphan",
        ])?;
        Ok(())
    }

    fn start_daemon(&mut self) -> Result<()> {
        self.daemon = Some(self.runtime.block_on(
            RunningDaemon::start_without_managed_resource_adapters(self.paths.clone()),
        )?);
        Ok(())
    }

    fn stop_daemon(&mut self) -> Result<()> {
        if let Some(daemon) = self.daemon.take() {
            self.runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(3), daemon.shutdown()).await
            })??;
        }
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        let daemon = self.stop_daemon();
        let system = self.cleanup_system();
        match (daemon, system) {
            (Err(error), Err(cleanup)) => {
                Err(error.context(format!("system cleanup also failed: {cleanup:#}")))
            }
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn cleanup_system(&mut self) -> Result<()> {
        if self.cleaned || !self.control_created {
            return Ok(());
        }
        if self.timer_attempted {
            root(&["/bin/sh", &format!("{CONTROL}/cleanup.sh")])?;
            require_missing()?;
            ensure!(
                !links()?.iter().any(|link| link["ifname"] == FOREIGN_LINK),
                "foreign fixture link remains"
            );
            for path in DNS_PATHS.into_iter().chain([RESOLVED_RUNTIME_MASK]) {
                ensure!(
                    !fs::path_entry_exists(Utf8Path::new(path))?,
                    "DNS fixture path remains: {path}"
                );
            }
            root(&[
                "/usr/bin/systemctl",
                "stop",
                &format!("{CLEANUP_UNIT}.timer"),
            ])?;
        }
        root(&["/usr/bin/rm", "-rf", "--", CONTROL])?;
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for DnsFixture {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            let _reported = writeln!(
                io::stderr().lock(),
                "DNS system fixture cleanup failed: {error:#}"
            );
        }
    }
}

fn require_resolved_stopped() -> Result<()> {
    require_resolved_inactive()?;
    assert!(link("pv0")?.is_none());
    Ok(())
}

fn require_resolved_inactive() -> Result<()> {
    assert_eq!(
        service_property("systemd-resolved.service", "ActiveState")?,
        "inactive"
    );
    assert_eq!(service_property("systemd-resolved.service", "Job")?, "");
    assert_eq!(
        text(checked(
            "/usr/bin/busctl",
            &[
                "--system",
                "call",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                "s",
                "org.freedesktop.resolve1",
            ]
        )?)?
        .trim(),
        "b false"
    );
    Ok(())
}

fn require_owned_endpoint() -> Result<()> {
    let addresses: Vec<Value> = serde_json::from_slice(
        &checked(
            "/usr/sbin/ip",
            &["-json", "-details", "address", "show", "dev", "pv0"],
        )?
        .stdout,
    )?;
    let observed = addresses.first().context("owned DNS link missing")?;
    ensure!(
        observed["ifname"] == "pv0"
            && observed["ifalias"] == "Managed by PV"
            && observed["linkinfo"]["info_kind"] == "dummy"
            && observed["flags"]
                .as_array()
                .is_some_and(|flags| flags.iter().any(|flag| flag == "UP")),
        "owned DNS link identity or up flag differs: {observed}"
    );
    let address = observed["addr_info"]
        .as_array()
        .context("DNS link addresses missing")?
        .iter()
        .find(|address| address["family"] == "inet" && address["local"] == "127.0.0.2")
        .context("DNS endpoint address missing")?;
    ensure!(
        address["prefixlen"] == 32
            && address["scope"] == "global"
            && address["valid_life_time"] == u32::MAX
            && address["preferred_life_time"] == u32::MAX,
        "DNS endpoint must be global /32 with infinite lifetimes: {address}"
    );
    let routes = endpoint_routes()?;
    let route = routes.first().context("DNS endpoint local route missing")?;
    ensure!(
        route["type"] == "local"
            && route["protocol"] == "kernel"
            && route["scope"] == "host"
            && route["prefsrc"] == "127.0.0.2"
            && route.get("gateway").is_none()
            && route.get("nexthops").is_none(),
        "DNS endpoint kernel local route differs: {route}"
    );
    Ok(())
}

fn create_owned_endpoint(address_options: &[&str]) -> Result<()> {
    root(&[
        "/usr/sbin/ip",
        "link",
        "add",
        "name",
        "pv0",
        "type",
        "dummy",
    ])?;
    root(&[
        "/usr/sbin/ip",
        "link",
        "set",
        "dev",
        "pv0",
        "alias",
        "Managed by PV",
    ])?;
    let mut command = vec![
        "/usr/sbin/ip",
        "address",
        "add",
        "127.0.0.2/32",
        "dev",
        "pv0",
    ];
    command.extend_from_slice(address_options);
    root(&command)?;
    root(&["/usr/sbin/ip", "link", "set", "dev", "pv0", "up"])?;
    Ok(())
}

fn endpoint_routes() -> Result<Vec<Value>> {
    let routes: Vec<Value> = serde_json::from_slice(
        &checked(
            "/usr/sbin/ip",
            &["-json", "-details", "-4", "route", "show", "table", "local"],
        )?
        .stdout,
    )?;
    Ok(routes
        .into_iter()
        .filter(|route| {
            route["dev"] == "pv0"
                && matches!(route["dst"].as_str(), Some("127.0.0.2" | "127.0.0.2/32"))
        })
        .collect())
}

fn foreign_owned_fact(address: bool) -> Result<Value> {
    if address {
        let links: Vec<Value> = serde_json::from_slice(
            &checked("/usr/sbin/ip", &["-json", "address", "show", "dev", "pv0"])?.stdout,
        )?;
        links
            .first()
            .and_then(|link| link["addr_info"].as_array())
            .and_then(|addresses| {
                addresses
                    .iter()
                    .find(|address| address["local"] == "192.0.2.2")
            })
            .cloned()
            .context("foreign address on pv0 missing")
    } else {
        let routes: Vec<Value> = serde_json::from_slice(
            &checked(
                "/usr/sbin/ip",
                &["-json", "-4", "route", "show", "dev", "pv0"],
            )?
            .stdout,
        )?;
        routes
            .into_iter()
            .find(|route| route["dst"] == "198.51.100.0/24" && route["protocol"] == "static")
            .context("foreign static route on pv0 missing")
    }
}

fn owned_installation_facts() -> Result<Value> {
    let observed = link("pv0")?.context("owned DNS link missing")?;
    let mut enabled_links = Vec::new();
    for path in &DNS_PATHS[3..5] {
        let path = Utf8Path::new(path);
        enabled_links.push(if fs::path_entry_exists(path)? {
            Some(fs::read_link(path)?)
        } else {
            None
        });
    }
    Ok(json!({
        "unit": fs::read_to_string(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH))?,
        "enabled_links": enabled_links,
        "link_index": observed["ifindex"],
        "link_alias": observed["ifalias"],
    }))
}

fn foreign_external_facts(address: bool) -> Result<Value> {
    let link = link(FOREIGN_LINK)?.context("external link missing")?;
    ensure!(link["ifalias"] == "Foreign DNS fixture");
    wait_for_resolved_link(FOREIGN_LINK)?;
    let domains = text(checked("/usr/bin/resolvectl", &["domain", FOREIGN_LINK])?)?;
    ensure!(
        !domains.split_whitespace().any(|word| {
            let domain = word.trim_start_matches('~').trim_end_matches('.');
            domain == "test" || domain.ends_with(".test")
        }),
        "external endpoint conflict masked by a test domain: {domains}"
    );
    let routes: Vec<Value> = serde_json::from_slice(
        &checked(
            "/usr/sbin/ip",
            &["-json", "-4", "route", "show", "dev", FOREIGN_LINK],
        )?
        .stdout,
    )?;
    let mut facts = json!({
        "index": link["ifindex"],
        "alias": link["ifalias"],
        "domains": domains,
        "static_route": routes.iter().find(|route| route["dst"] == "203.0.113.0/24" && route["protocol"] == "static").context("foreign static route missing")?,
    });
    if address {
        let links: Vec<Value> = serde_json::from_slice(
            &checked(
                "/usr/sbin/ip",
                &["-json", "address", "show", "dev", FOREIGN_LINK],
            )?
            .stdout,
        )?;
        facts["endpoint_address"] = links
            .first()
            .and_then(|link| link["addr_info"].as_array())
            .and_then(|addresses| {
                addresses
                    .iter()
                    .find(|address| address["local"] == "127.0.0.2")
            })
            .context("external endpoint address missing")?
            .clone();
    } else {
        facts["endpoint_route"] = routes
            .into_iter()
            .find(|route| {
                matches!(route["dst"].as_str(), Some("127.0.0.2" | "127.0.0.2/32"))
                    && route["protocol"] == "static"
            })
            .context("external endpoint static route missing")?;
    }
    Ok(facts)
}

fn require_current(port: u16) -> Result<()> {
    let expected = ResolverConfig::new(port);
    let state =
        platform::inspect_resolver_file(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH), Some(&expected));
    ensure!(
        matches!(state, ResolverFileState::Current { port: observed, .. } if observed == port),
        "{state:?}"
    );
    assert_eq!(PrivilegedHelperClient.inspect_dns(Some(&expected))?, state);
    Ok(())
}

fn require_owned_resources_removed() -> Result<()> {
    for path in DNS_PATHS.into_iter().filter(|&path| path != DNSSEC_FILE) {
        ensure!(
            !fs::path_entry_exists(Utf8Path::new(path))?,
            "PV DNS path remains: {path}"
        );
    }
    assert!(link("pv0")?.is_none());
    require_units_absent(&["pv-dns.service"])
}

fn require_unsafe_unit_refused(port: u16) -> Result<()> {
    let expected = ResolverConfig::new(port);
    for state in [
        platform::inspect_resolver_file(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH), Some(&expected)),
        PrivilegedHelperClient.inspect_dns(Some(&expected))?,
    ] {
        ensure!(
            matches!(state, ResolverFileState::Unreadable { .. }),
            "{state:?}"
        );
    }
    Ok(())
}

fn wait_until_current(port: u16) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match require_current(port) {
            Ok(()) => return Ok(()),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn require_missing() -> Result<()> {
    let state = platform::inspect_resolver_file(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH), None);
    ensure!(
        matches!(state, ResolverFileState::Missing { .. }),
        "{state:?}"
    );
    for path in DNS_PATHS.into_iter().filter(|path| *path != DNSSEC_FILE) {
        ensure!(
            !fs::path_entry_exists(Utf8Path::new(path))?,
            "DNS integration path remains after removal: {path}"
        );
    }
    Ok(())
}

fn require_conflict(port: u16) -> Result<()> {
    let config = ResolverConfig::new(port);
    let state =
        platform::inspect_resolver_file(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH), Some(&config));
    ensure!(
        matches!(state, ResolverFileState::Conflict { .. }),
        "{state:?}"
    );
    for result in [
        PrivilegedHelperClient.apply_dns(&config),
        PrivilegedHelperClient.remove_dns(),
    ] {
        ensure!(
            matches!(result, Err(PlatformError::SystemIntegration(_))),
            "mutation did not reject the native conflict: {result:?}"
        );
    }
    Ok(())
}

fn require_lookup(hostname: &str) -> Result<()> {
    require_lookup_answer(hostname, "127.0.0.1")
}

fn require_lookup_answer(hostname: &str, expected: &str) -> Result<()> {
    let output = checked(
        "/usr/bin/timeout",
        &[
            "5s",
            "/usr/bin/python3",
            "-c",
            "import socket,sys; print(','.join(sorted({result[4][0] for result in socket.getaddrinfo(sys.argv[1], None, socket.AF_INET)})))",
            hostname,
        ],
    )?;
    assert_eq!(text(output)?.trim(), expected);
    Ok(())
}

async fn require_production_transports(port: u16, hostname: &str) -> Result<()> {
    let name = Name::from_ascii(format!("{hostname}."))?;
    let mut query = Message::new(42, MessageType::Query, OpCode::Query);
    query.metadata.recursion_desired = true;
    query.add_query(Query::query(name.clone(), RecordType::A));
    let query = query.to_bytes()?;
    for tcp in [false, true] {
        let bytes: Vec<u8> = timeout(Duration::from_secs(2), async {
            let address = (Ipv4Addr::new(127, 0, 0, 2), port);
            if tcp {
                let mut stream = TcpStream::connect(address).await?;
                stream
                    .write_all(&u16::try_from(query.len())?.to_be_bytes())
                    .await?;
                stream.write_all(&query).await?;
                let length = stream.read_u16().await?;
                let mut bytes = vec![0; usize::from(length)];
                stream.read_exact(&mut bytes).await?;
                Ok::<_, anyhow::Error>(bytes)
            } else {
                let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
                socket.connect(address).await?;
                socket.send(&query).await?;
                let mut bytes = vec![0; 4096];
                let length = socket.recv(&mut bytes).await?;
                bytes.truncate(length);
                Ok(bytes)
            }
        })
        .await??;
        let response = Message::from_vec(&bytes)?;
        assert_eq!(response.metadata.id, 42);
        assert_eq!(response.metadata.response_code, ResponseCode::NoError);
        assert!(!response.metadata.truncation);
        assert_eq!(response.answers.len(), 1);
        assert_eq!(response.answers[0].name, name);
        assert_eq!(response.answers[0].data, RData::A(A::new(127, 0, 0, 1)));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum DnsResponder {
    Pv,
    Foreign,
}

impl DnsResponder {
    fn label(self) -> &'static str {
        match self {
            Self::Pv => "pv",
            Self::Foreign => "foreign",
        }
    }
}

type DnsEvent = (&'static str, &'static str, String);

fn record_dns_queries(
    request: &[u8],
    responder: DnsResponder,
    transport: &'static str,
    events: &mpsc::Sender<DnsEvent>,
) -> Result<()> {
    for query in Message::from_vec(request)?.queries {
        if query.query_type() == RecordType::A {
            events.try_send((
                responder.label(),
                transport,
                query.name().to_string().to_ascii_lowercase(),
            ))?;
        }
    }
    Ok(())
}

fn controlled_dns_response(request: &[u8], responder: DnsResponder, udp: bool) -> Result<Vec<u8>> {
    let request = Message::from_vec(request)?;
    let mut response = Message::response(request.metadata.id, request.metadata.op_code);
    response.metadata.recursion_desired = request.metadata.recursion_desired;
    response.metadata.authoritative = true;
    response.metadata.truncation = udp && matches!(responder, DnsResponder::Pv);
    response.add_queries(request.queries.iter().cloned());
    let answer = match responder {
        DnsResponder::Pv => A::new(127, 0, 0, 1),
        DnsResponder::Foreign => A::new(192, 0, 2, 123),
    };
    for query in &request.queries {
        if query.query_type() == RecordType::A && !response.metadata.truncation {
            response.add_answer(Record::from_rdata(
                query.name().clone(),
                1,
                RData::A(answer),
            ));
        }
    }
    Ok(response.to_bytes()?)
}

async fn serve_probe_udp(
    socket: UdpSocket,
    responder: DnsResponder,
    events: mpsc::Sender<DnsEvent>,
) -> Result<()> {
    let mut bytes = [0; 4096];
    loop {
        let (length, peer) = socket.recv_from(&mut bytes).await?;
        let request = &bytes[..length];
        let transport = if matches!(responder, DnsResponder::Pv) {
            "udp-tc1"
        } else {
            "udp"
        };
        record_dns_queries(request, responder, transport, &events)?;
        socket
            .send_to(&controlled_dns_response(request, responder, true)?, peer)
            .await?;
        let sent = if matches!(responder, DnsResponder::Pv) {
            "udp-tc1-sent"
        } else {
            "udp-sent"
        };
        record_dns_queries(request, responder, sent, &events)?;
    }
}

async fn serve_probe_tcp(
    listener: TcpListener,
    responder: DnsResponder,
    events: mpsc::Sender<DnsEvent>,
) -> Result<()> {
    loop {
        let (mut stream, _peer) = listener.accept().await?;
        timeout(Duration::from_secs(2), async {
            let length = stream.read_u16().await?;
            let mut request = vec![0; usize::from(length)];
            stream.read_exact(&mut request).await?;
            record_dns_queries(&request, responder, "tcp", &events)?;
            let response = controlled_dns_response(&request, responder, false)?;
            stream
                .write_all(&u16::try_from(response.len())?.to_be_bytes())
                .await?;
            stream.write_all(&response).await?;
            record_dns_queries(&request, responder, "tcp-sent", &events)?;
            Ok::<_, anyhow::Error>(())
        })
        .await??;
    }
}

async fn controlled_dns_probes(
    port: u16,
    foreign_port: u16,
    hostname: String,
    unrelated: String,
) -> Result<Vec<DnsEvent>> {
    let pv_udp = UdpSocket::bind((Ipv4Addr::new(127, 0, 0, 2), port)).await?;
    let pv_tcp = TcpListener::bind((Ipv4Addr::new(127, 0, 0, 2), port)).await?;
    let foreign_udp = UdpSocket::bind((Ipv4Addr::new(192, 0, 2, 53), foreign_port)).await?;
    let foreign_tcp = TcpListener::bind((Ipv4Addr::new(192, 0, 2, 53), foreign_port)).await?;
    let (events, mut receiver) = mpsc::channel(64);
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_probe_udp(pv_udp, DnsResponder::Pv, events.clone()));
    tasks.spawn(serve_probe_tcp(pv_tcp, DnsResponder::Pv, events.clone()));
    tasks.spawn(serve_probe_udp(
        foreign_udp,
        DnsResponder::Foreign,
        events.clone(),
    ));
    tasks.spawn(serve_probe_tcp(foreign_tcp, DnsResponder::Foreign, events));
    let expected_test = format!("{hostname}.");
    let expected_foreign = format!("{unrelated}.");
    let result: Result<()> = async {
        wait_for_resolved_link(FOREIGN_LINK)?;
        let foreign_dns = format!("192.0.2.53:{foreign_port}");
        for arguments in [
            vec!["dns", FOREIGN_LINK, &foreign_dns],
            vec!["domain", FOREIGN_LINK, "~pv-foreign.example"],
            vec!["default-route", FOREIGN_LINK, "no"],
            vec!["dnssec", FOREIGN_LINK, "no"],
            vec!["dnsovertls", FOREIGN_LINK, "no"],
            vec!["llmnr", FOREIGN_LINK, "no"],
            vec!["mdns", FOREIGN_LINK, "no"],
        ] {
            let mut command = vec!["/usr/bin/resolvectl"];
            command.extend(arguments);
            root(&command)?;
        }
        wait_for_dns_scope("pv0")?;
        wait_for_dns_scope(FOREIGN_LINK)?;
        tokio::task::spawn_blocking(move || {
            require_lookup(&hostname)?;
            require_lookup_answer(&unrelated, "192.0.2.123")
        })
        .await??;
        Ok(())
    }
    .await;
    tasks.abort_all();
    let mut errors = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(Err(error)) => errors.push(error.to_string()),
            Err(error) if !error.is_cancelled() => errors.push(error.to_string()),
            Ok(Ok(())) | Err(_) => {}
        }
    }
    let mut trace = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        trace.push(event);
    }
    let details = format!("DNS trace: {trace:?}");
    let result = result.and_then(|()| {
        let udp = trace
            .iter()
            .position(|(server, transport, name)| {
                *server == "pv" && *transport == "udp-tc1" && name == &expected_test
            })
            .context("fresh truncated UDP query missing")?;
        let tcp = trace
            .iter()
            .position(|(server, transport, name)| {
                *server == "pv" && *transport == "tcp" && name == &expected_test
            })
            .context("same-name TCP retry missing")?;
        ensure!(udp < tcp, "TCP did not follow truncated UDP");
        ensure!(
            trace.iter().any(|(server, transport, name)| *server == "pv"
                && *transport == "udp-tc1-sent"
                && name == &expected_test),
            "truncated UDP reply did not finish sending"
        );
        ensure!(
            trace.iter().any(|(server, transport, name)| *server == "pv"
                && *transport == "tcp-sent"
                && name == &expected_test),
            "TCP retry reply did not finish sending"
        );
        ensure!(
            trace
                .iter()
                .any(|(server, _, name)| *server == "foreign" && name == &expected_foreign),
            "unrelated query did not reach eligible foreign scope"
        );
        ensure!(
            !trace.iter().any(
                |(server, _, name)| (*server == "foreign" && name == &expected_test)
                    || (*server == "pv" && name == &expected_foreign)
            ),
            "query leaked across routing domains"
        );
        Ok(trace)
    });
    let cleanup = if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("DNS responder failures: {errors:?}"))
    };
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("{details}; cleanup also failed: {cleanup:#}")))
        }
        (Err(error), _) | (_, Err(error)) => Err(error.context(details)),
        (Ok(trace), Ok(())) => Ok(trace),
    }
}

fn wait_for_dns_scope(name: &str) -> Result<()> {
    let index = link(name)?.context("DNS scope link missing")?["ifindex"]
        .as_u64()
        .context("DNS scope index missing")?
        .to_string();
    let object: Value = serde_json::from_slice(
        &checked(
            "/usr/bin/busctl",
            &[
                "--system",
                "--json=short",
                "call",
                "org.freedesktop.resolve1",
                "/org/freedesktop/resolve1",
                "org.freedesktop.resolve1.Manager",
                "GetLink",
                "i",
                &index,
            ],
        )?
        .stdout,
    )?;
    let object = object["data"][0]
        .as_str()
        .context("resolved link path missing")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let property = text(checked(
            "/usr/bin/busctl",
            &[
                "--system",
                "get-property",
                "org.freedesktop.resolve1",
                object,
                "org.freedesktop.resolve1.Link",
                "ScopesMask",
            ],
        )?)?;
        let mask = property
            .trim()
            .strip_prefix("t ")
            .context("DNS scope mask type differs")?
            .parse::<u64>()?;
        if mask & 1 != 0 {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "{name} has no effective DNS scope: {mask}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn canonical_observation(phase: &str, port: u16) -> Result<()> {
    let route: Vec<Value> = serde_json::from_slice(
        &checked(
            "/usr/sbin/ip",
            &["-json", "-4", "route", "get", "127.0.0.1"],
        )?
        .stdout,
    )?;
    let observed = route.first().context("canonical loopback route missing")?;
    ensure!(
        observed["type"] == "local"
            && observed["dev"] == "lo"
            && observed["prefsrc"] == "127.0.0.1",
        "canonical route differs: {observed}"
    );
    let connection = StdTcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_secs(2),
    )?;
    assert_eq!(connection.local_addr()?.ip(), Ipv4Addr::LOCALHOST);
    let hostname: Value = serde_json::from_slice(&checked("/usr/bin/timeout", &["5s", "/usr/bin/python3", "-c", "import json,socket\nname=socket.gethostname()\ntry: result={'hostname':name,'addresses':sorted({row[4][0] for row in socket.getaddrinfo(name,None,socket.AF_INET)})}\nexcept socket.gaierror as error: result={'hostname':name,'error':str(error)}\nprint(json.dumps(result))"])?.stdout)?;
    writeln!(
        io::stdout().lock(),
        "canonical loopback {phase}: {}",
        json!({"route":route,"connection_source":connection.local_addr()?.to_string(),"own_hostname":hostname})
    )?;
    Ok(())
}

fn links() -> Result<Vec<Value>> {
    Ok(serde_json::from_slice(
        &checked("/usr/sbin/ip", &["-json", "link", "show"])?.stdout,
    )?)
}

fn link(name: &str) -> Result<Option<Value>> {
    Ok(links()?.into_iter().find(|link| link["ifname"] == name))
}

fn wait_for_resolved_link(link: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match checked("/usr/bin/resolvectl", &["status", link]) {
            Ok(_) => return Ok(()),
            Err(error) if Instant::now() >= deadline => {
                return Err(error.context("resolved did not observe the fixture link"));
            }
            Err(_) => thread::sleep(Duration::from_millis(25)),
        }
    }
}
