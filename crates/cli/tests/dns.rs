use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::net::{Ipv4Addr, TcpListener, UdpSocket};
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::assert_debug_snapshot;
use platform::ResolverConfig;
use state::{Database, PortOwner, PortRequest, PvPaths, StateError};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: RefCell<PathBuf>,
    resolver_path: PathBuf,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path, resolver_path: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: RefCell::new(current_dir.as_std_path().to_path_buf()),
            resolver_path: resolver_path.as_std_path().to_path_buf(),
        }
    }
}

impl Environment for TestEnvironment {
    fn var_os(&self, _key: &str) -> Option<OsString> {
        None
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.current_dir.borrow().clone())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(PathBuf::from("/bin/pv"))
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn read_line(&self) -> io::Result<String> {
        Ok(String::new())
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn resolver_test_path(&self) -> PathBuf {
        self.resolver_path.clone()
    }
}

#[test]
fn dns_install_prepares_resolver_config_without_touching_system_path() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_resolver_path = tempdir.path().join("etc/resolver/test");
    let environment = TestEnvironment::new(&home, &current_dir, &system_resolver_path);

    let output = run_pv(&["dns:install"], &environment)?;
    let prepared_path = pv_paths(&home).resolver_config();
    let prepared_config = read_required_file(&prepared_path)?;
    let parsed_config = ResolverConfig::parse(&prepared_config)
        .ok_or_else(|| anyhow::anyhow!("prepared resolver config did not parse"))?;
    let system_resolver_config = read_optional_file(&system_resolver_path)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(!output.stdout.is_empty());
    assert_no_manual_guidance(&output.stdout);
    assert!(output.stderr.is_empty());
    assert!(system_resolver_config.is_none());
    with_normalized_tempdir(tempdir.path(), || {
        let mut settings = insta::Settings::clone_current();
        settings.add_filter(
            r"DNS resolver port: [0-9]+",
            "DNS resolver port: <dns-port>",
        );
        settings.add_filter(r"port [0-9]+", "port <dns-port>");
        settings.add_filter(r"port: [0-9]+", "port: <dns-port>");
        settings.bind(|| {
            assert_debug_snapshot!((
                output,
                prepared_path,
                prepared_config,
                parsed_config,
                system_resolver_config,
            ));
        });
    });

    Ok(())
}

#[test]
fn dns_install_reuses_persisted_dns_port_even_when_it_is_bound() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_resolver_path = tempdir.path().join("etc/resolver/test");
    let environment = TestEnvironment::new(&home, &current_dir, &system_resolver_path);
    let paths = pv_paths(&home);
    let (dns_port, _tcp_listener, _udp_socket) = bind_loopback_tcp_udp_pair()?;
    let mut database = Database::open(&paths)?;
    let seeded_assignment = database.assign_port(
        PortRequest::dns(dns_port, dns_port, dns_port),
        |candidate| candidate == dns_port,
    )?;
    drop(database);

    let output = run_pv(&["dns:install"], &environment)?;
    let prepared_config = read_required_file(&paths.resolver_config())?;
    let parsed_config = ResolverConfig::parse(&prepared_config)
        .ok_or_else(|| anyhow::anyhow!("prepared resolver config did not parse"))?;
    let assignments = Database::open(&paths)?.assigned_ports()?;
    let dns_assignment = assignments
        .into_iter()
        .find(|assignment| assignment.owner == PortOwner::Dns)
        .ok_or_else(|| anyhow::anyhow!("missing DNS port assignment"))?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert_eq!(seeded_assignment.port, dns_port);
    assert_eq!(parsed_config.port, dns_port);
    assert_eq!(dns_assignment.port, dns_port);

    Ok(())
}

#[test]
fn dns_install_reports_non_pv_owned_system_resolver_conflict() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_resolver_path = tempdir.path().join("etc/resolver/test");
    let environment = TestEnvironment::new(&home, &current_dir, &system_resolver_path);
    let conflict_config = "nameserver 127.0.0.1\nport 35353\n";
    write_file(&system_resolver_path, conflict_config)?;

    let output = run_pv(&["dns:install"], &environment)?;
    let prepared_path = pv_paths(&home).resolver_config();
    let prepared_config = read_required_file(&prepared_path)?;
    let system_after_install = read_required_file(&system_resolver_path)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert_no_manual_guidance(&output.stdout);
    assert!(output.stderr.is_empty());
    assert_eq!(system_after_install, conflict_config);
    with_normalized_tempdir(tempdir.path(), || {
        let mut settings = insta::Settings::clone_current();
        settings.add_filter(
            r"DNS resolver port: [0-9]+",
            "DNS resolver port: <dns-port>",
        );
        settings.add_filter(r"port [0-9]+", "port <dns-port>");
        settings.bind(|| {
            assert_debug_snapshot!((
                output,
                prepared_path,
                prepared_config,
                system_resolver_path,
                system_after_install,
            ));
        });
    });

    Ok(())
}

#[test]
fn dns_status_reports_prepared_and_system_resolver_states() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_resolver_path = tempdir.path().join("etc/resolver/test");
    let environment = TestEnvironment::new(&home, &current_dir, &system_resolver_path);
    let prepared_path = pv_paths(&home).resolver_config();
    let current_config = ResolverConfig::new(35353).render();
    let stale_config = ResolverConfig::new(45000).render();
    let conflict_config = "nameserver 127.0.0.1\nport 35353\n";

    let missing = run_pv(&["dns:status"], &environment)?;
    let prepared_after_missing = read_optional_file(&prepared_path)?;
    let system_after_missing = read_optional_file(&system_resolver_path)?;

    write_file(&prepared_path, &current_config)?;
    let prepared_only = run_pv(&["dns:status"], &environment)?;

    write_file(&system_resolver_path, &current_config)?;
    let current = run_pv(&["dns:status"], &environment)?;

    write_file(&system_resolver_path, &stale_config)?;
    let stale = run_pv(&["dns:status"], &environment)?;

    write_file(&system_resolver_path, conflict_config)?;
    let conflict = run_pv(&["dns:status"], &environment)?;

    assert_eq!(missing.exit_code, ExitCode::SUCCESS);
    assert_eq!(prepared_only.exit_code, ExitCode::SUCCESS);
    assert_eq!(current.exit_code, ExitCode::SUCCESS);
    assert_eq!(stale.exit_code, ExitCode::SUCCESS);
    assert_eq!(conflict.exit_code, ExitCode::SUCCESS);
    assert!(missing.stderr.is_empty());
    assert!(prepared_only.stderr.is_empty());
    assert!(current.stderr.is_empty());
    assert!(stale.stderr.is_empty());
    assert!(conflict.stderr.is_empty());
    assert!(prepared_after_missing.is_none());
    assert!(system_after_missing.is_none());
    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((
            missing,
            prepared_after_missing,
            system_after_missing,
            prepared_only,
            current,
            stale,
            conflict,
        ));
    });

    Ok(())
}

#[test]
fn dns_uninstall_removes_prepared_config_and_defers_pv_owned_system_removal() -> anyhow::Result<()>
{
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_resolver_path = tempdir.path().join("etc/resolver/test");
    let environment = TestEnvironment::new(&home, &current_dir, &system_resolver_path);
    let prepared_path = pv_paths(&home).resolver_config();
    let resolver_config = ResolverConfig::new(35353).render();
    write_file(&prepared_path, &resolver_config)?;
    write_file(&system_resolver_path, &resolver_config)?;

    let output = run_pv(&["dns:uninstall"], &environment)?;
    let prepared_after_uninstall = read_optional_file(&prepared_path)?;
    let system_after_uninstall = read_required_file(&system_resolver_path)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert_no_manual_guidance(&output.stdout);
    assert!(output.stderr.is_empty());
    assert!(prepared_after_uninstall.is_none());
    assert_eq!(system_after_uninstall, resolver_config);
    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((
            output,
            prepared_path,
            prepared_after_uninstall,
            system_resolver_path,
            system_after_uninstall,
        ));
    });

    Ok(())
}

#[test]
fn dns_uninstall_fails_when_system_resolver_cannot_be_inspected() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("work");
    let system_resolver_path = tempdir.path().join("etc/resolver/test");
    let environment = TestEnvironment::new(&home, &current_dir, &system_resolver_path);
    let prepared_path = pv_paths(&home).resolver_config();
    let resolver_config = ResolverConfig::new(35353).render();
    write_file(&prepared_path, &resolver_config)?;
    create_dir(&system_resolver_path)?;

    let output = run_pv(&["dns:uninstall"], &environment)?;
    let prepared_after_uninstall = read_optional_file(&prepared_path)?;

    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert_no_manual_guidance(&output.stdout);
    assert!(output.stderr.is_empty());
    assert!(prepared_after_uninstall.is_none());
    with_normalized_tempdir(tempdir.path(), || {
        assert_debug_snapshot!((output, prepared_path, prepared_after_uninstall));
    });

    Ok(())
}

#[derive(Debug)]
struct RunOutput {
    exit_code: ExitCode,
    stdout: String,
    stderr: String,
}

fn run_pv(args: &[&str], environment: &impl Environment) -> anyhow::Result<RunOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let args = std::iter::once("pv").chain(args.iter().copied());
    let exit_code = run_with_environment(args, environment, &mut stdout, &mut stderr)?;

    Ok(RunOutput {
        exit_code,
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8(stderr)?,
    })
}

fn pv_paths(home: &Utf8Path) -> PvPaths {
    PvPaths::for_home(home.to_path_buf())
}

fn read_required_file(path: &Utf8Path) -> anyhow::Result<String> {
    read_optional_file(path)?
        .ok_or_else(|| anyhow::anyhow!("expected fixture file to exist: {path}"))
}

fn read_optional_file(path: &Utf8Path) -> anyhow::Result<Option<String>> {
    match state::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn write_file(path: &Utf8Path, content: &str) -> anyhow::Result<()> {
    state::fs::write_sensitive_file(path, content)?;

    Ok(())
}

fn bind_loopback_tcp_udp_pair() -> anyhow::Result<(u16, TcpListener, UdpSocket)> {
    for _attempt in 0..100 {
        let tcp_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = tcp_listener.local_addr()?.port();
        let Ok(udp_socket) = UdpSocket::bind((Ipv4Addr::LOCALHOST, port)) else {
            continue;
        };

        return Ok((port, tcp_listener, udp_socket));
    }

    Err(anyhow::anyhow!(
        "could not bind a loopback TCP/UDP port pair"
    ))
}

fn assert_no_manual_guidance(output: &str) {
    for pattern in [
        "sudo",
        "Move or remove",
        "move",
        "remove it manually",
        "sudo rm",
        "sudo install",
    ] {
        assert!(
            !output.contains(pattern),
            "output contains unsafe manual guidance `{pattern}`: {output}"
        );
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI DNS tests create an unreadable resolver fixture path"
)]
fn create_dir(path: &Utf8Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

fn with_normalized_tempdir(tempdir: &Utf8Path, assertion: impl FnOnce()) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(assertion);
}
