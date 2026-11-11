use platform::clock::{self, real};
use std::io::{self, Write as _};
use std::net::{Ipv4Addr, ToSocketAddrs};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use camino::Utf8Path;
use daemon::RunningDaemon;
use platform::{
    PlatformError, PrivilegedHelperClient, ResolverConfig, ResolverFileState,
    SYSTEM_RESOLVER_TEST_PATH,
};
use state::{Database, PortOwner, PvPaths, fs};
use tokio::runtime::{Builder, Runtime};

use super::{
    HelperFixture, checked, checksum, require_units_absent, root, root_text, service_property,
    text, with_helper_fixture,
};

const FOREIGN_LINK: &str = "pv-dns-foreign";
const DNS_PATHS: [&str; 5] = [
    SYSTEM_RESOLVER_TEST_PATH,
    "/etc/systemd/system/pv-dns.service.candidate",
    "/etc/systemd/system/pv-dns.service.d",
    "/etc/systemd/system/systemd-resolved.service.wants/pv-dns.service",
    "/run/systemd/system/systemd-resolved.service.wants/pv-dns.service",
];

#[test]
#[ignore = "requires PV_LINUX_SYSTEM_TESTS=1, the built helper and a disposable systemd machine with resolved and dummy links"]
fn systemd_dns_routes_test_reapplies_after_restart_changes_port_and_refuses_conflicts() -> Result<()>
{
    with_helper_fixture(with_dns_fixture)
}

fn with_dns_fixture(helper_fixture: &mut HelperFixture, helper: &Utf8Path) -> Result<()> {
    preflight()?;
    let paths = PvPaths::for_home(helper_fixture.scratch.path().join("dns-home"));
    let mut fixture = DnsFixture {
        paths,
        runtime: Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?,
        daemon: None,
        cleaned: false,
    };
    let result = fixture.exercise(helper_fixture, helper);
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
    for path in DNS_PATHS {
        ensure!(
            !fs::path_entry_exists(Utf8Path::new(path))?,
            "refusing existing DNS path {path}"
        );
    }
    ensure!(
        link("pv0")?.is_none() && link(FOREIGN_LINK)?.is_none(),
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
    require_units_absent(&["pv-dns.service"])?;
    Ok(())
}

struct DnsFixture {
    paths: PvPaths,
    runtime: Runtime,
    daemon: Option<RunningDaemon>,
    cleaned: bool,
}

impl DnsFixture {
    fn exercise(&mut self, helper_fixture: &mut HelperFixture, helper: &Utf8Path) -> Result<()> {
        helper_fixture.install(helper, &checksum(helper)?)?;
        let first_port = pv_fake::seed_dns_port(&self.paths)?;
        let suffix = helper_fixture
            .scratch
            .path()
            .file_name()
            .context("fixture directory name missing")?
            .trim_start_matches('.');
        self.start_daemon()?;
        PrivilegedHelperClient.apply_dns(&ResolverConfig::new(first_port))?;
        require_current(first_port)?;
        require_lookup(&format!("initial.{suffix}.pv-dns-system.test"))?;
        assert_eq!(
            fs::read_link(Utf8Path::new(DNS_PATHS[3]))?,
            Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH)
        );
        assert_eq!(
            root_text(&["/usr/bin/stat", "-c", "%u:%g:%a", SYSTEM_RESOLVER_TEST_PATH])?.trim(),
            "0:0:644"
        );
        root(&["/usr/bin/systemctl", "restart", "systemd-resolved.service"])?;
        wait_until_current(first_port)?;
        require_lookup(&format!("restarted.{suffix}.pv-dns-system.test"))?;

        self.stop_daemon()?;
        Database::open(&self.paths)?.release_port(PortOwner::Dns)?;
        let second_port = pv_fake::seed_dns_port(&self.paths)?;
        assert_ne!(second_port, first_port);
        self.start_daemon()?;
        PrivilegedHelperClient.apply_dns(&ResolverConfig::new(second_port))?;
        require_current(second_port)?;
        require_lookup(&format!("changed.{suffix}.pv-dns-system.test"))?;
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
        let before = link("pv0")?.context("foreign pv0 missing")?;
        require_conflict(second_port)?;
        let after = link("pv0")?.context("foreign pv0 was removed")?;
        assert_eq!(after, before);
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
        let config = ResolverConfig::new(second_port);
        PrivilegedHelperClient.apply_dns(&config)?;
        root(&["/usr/bin/resolvectl", "domain", FOREIGN_LINK, "~test"])?;
        let domains = text(checked("/usr/bin/resolvectl", &["domain", FOREIGN_LINK])?)?;
        assert!(matches!(
            platform::inspect_resolver_file(
                Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH),
                Some(&config)
            ),
            ResolverFileState::Conflict { .. }
        ));
        assert!(matches!(
            PrivilegedHelperClient.apply_dns(&config),
            Err(PlatformError::SystemIntegration(_))
        ));
        PrivilegedHelperClient.remove_dns()?;
        require_owned_resources_removed()?;
        assert_eq!(
            text(checked("/usr/bin/resolvectl", &["domain", FOREIGN_LINK])?)?,
            domains
        );
        root(&["/usr/bin/resolvectl", "revert", FOREIGN_LINK])?;
        root(&["/usr/sbin/ip", "link", "delete", "dev", FOREIGN_LINK])?;
        require_missing()?;
        Ok(())
    }

    fn start_daemon(&mut self) -> Result<()> {
        self.daemon = Some(self.runtime.block_on(
            RunningDaemon::start_without_managed_resource_adapters(
                self.paths.clone(),
                crate::support::test_host()?,
            ),
        )?);
        Ok(())
    }

    fn stop_daemon(&mut self) -> Result<()> {
        if let Some(daemon) = self.daemon.take() {
            self.runtime.block_on(async {
                real::timeout(Duration::from_secs(3), daemon.shutdown()).await
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
        if self.cleaned {
            return Ok(());
        }
        if service_property("pv-dns.service", "ActiveState")? != "inactive" {
            root(&["/usr/bin/systemctl", "stop", "pv-dns.service"])?;
        }
        for name in ["pv0", FOREIGN_LINK] {
            if link(name)?.is_some() {
                root(&["/usr/sbin/ip", "link", "delete", "dev", name])?;
            }
        }
        root(&[
            "/usr/bin/rm",
            "-f",
            "--",
            DNS_PATHS[0],
            DNS_PATHS[1],
            DNS_PATHS[3],
            DNS_PATHS[4],
        ])?;
        root(&["/usr/bin/systemctl", "daemon-reload"])?;
        root(&["/usr/bin/systemctl", "reset-failed"])?;
        require_missing()?;
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
    for path in DNS_PATHS {
        ensure!(
            !fs::path_entry_exists(Utf8Path::new(path))?,
            "PV DNS path remains: {path}"
        );
    }
    assert!(link("pv0")?.is_none());
    require_units_absent(&["pv-dns.service"])
}

fn wait_until_current(port: u16) -> Result<()> {
    let deadline = real::now() + Duration::from_secs(10);
    loop {
        match require_current(port) {
            Ok(()) => return Ok(()),
            Err(error) if real::now() >= deadline => return Err(error),
            Err(_) => clock::poll_blocking(Duration::from_millis(25)),
        }
    }
}

fn require_missing() -> Result<()> {
    let state = platform::inspect_resolver_file(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH), None);
    ensure!(
        matches!(state, ResolverFileState::Missing { .. }),
        "{state:?}"
    );
    for path in DNS_PATHS {
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
    let addresses: Vec<_> = (hostname, 0).to_socket_addrs()?.collect();
    ensure!(
        !addresses.is_empty()
            && addresses
                .iter()
                .all(|address| address.ip() == Ipv4Addr::LOCALHOST)
    );
    Ok(())
}

fn link(name: &str) -> Result<Option<(String, String)>> {
    let path = Utf8Path::new("/sys/class/net").join(name);
    if !path.is_dir() {
        return Ok(None);
    }
    Ok(Some((
        fs::read_to_string(&path.join("ifindex"))?,
        fs::read_to_string(&path.join("ifalias"))?,
    )))
}

fn wait_for_resolved_link(link: &str) -> Result<()> {
    let deadline = real::now() + Duration::from_secs(5);
    loop {
        match checked("/usr/bin/resolvectl", &["status", link]) {
            Ok(_) => return Ok(()),
            Err(error) if real::now() >= deadline => {
                return Err(error.context("resolved did not observe the fixture link"));
            }
            Err(_) => clock::poll_blocking(Duration::from_millis(25)),
        }
    }
}
