use std::io::{self, Write as _};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use camino::Utf8Path;
use daemon::RunningDaemon;
use platform::{
    PlatformError, PrivilegedHelperClient, ResolverConfig, ResolverFileState,
    SYSTEM_RESOLVER_TEST_PATH,
};
use serde_json::Value;
use state::{Database, PortOwner, PvPaths, fs};
use tokio::runtime::{Builder, Runtime};

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
        self.start_daemon()?;
        PrivilegedHelperClient.apply_dns(&ResolverConfig::new(first_port))?;
        require_current(first_port)?;
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
        require_lookup("initial.pv-dns-system.test")?;

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
        let hard_link = format!("{CONTROL}/unit-hard-link");
        root(&["/usr/bin/ln", SYSTEM_RESOLVER_TEST_PATH, &hard_link])?;
        require_unsafe_unit_refused(first_port)?;
        root(&["/usr/bin/rm", "--", &hard_link])?;
        require_current(first_port)?;

        root(&["/usr/bin/systemctl", "restart", "systemd-resolved.service"])?;
        wait_until_current(first_port)?;
        require_lookup("restarted.pv-dns-system.test")?;

        self.stop_daemon()?;
        Database::open(&self.paths)?.release_port(PortOwner::Dns)?;
        let second_port = pv_fake::seed_dns_port(&self.paths)?;
        assert_ne!(second_port, first_port);
        self.start_daemon()?;
        PrivilegedHelperClient.apply_dns(&ResolverConfig::new(second_port))?;
        require_current(second_port)?;
        require_lookup("changed-port.pv-dns-system.test")?;
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
        PrivilegedHelperClient.remove_dns()?;
        PrivilegedHelperClient.remove_dns()?;
        require_owned_resources_removed()?;
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
    assert!(link("pv0")?.is_none());
    Ok(())
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
    assert_eq!(text(output)?.trim(), "127.0.0.1");
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
