#![cfg(target_os = "macos")]

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::Environment;
use insta::assert_debug_snapshot;
use platform::{ResolverFileState, SYSTEM_RESOLVER_TEST_PATH};
use state::{Database, PortRequest, PvPaths};

use crate::support::run_pv;

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    resolver: RefCell<ResolverFileState>,
    terminal_width: Cell<Option<usize>>,
}

impl TestEnvironment {
    fn new(home: &Utf8Path) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            resolver: RefCell::new(ResolverFileState::Missing {
                path: SYSTEM_RESOLVER_TEST_PATH.into(),
            }),
            terminal_width: Cell::new(None),
        }
    }
}

impl Environment for TestEnvironment {
    fn inspect_low_ports(&self) -> Result<platform::LowPortInspection, platform::PlatformError> {
        Err(platform::PlatformError::PrivilegedHelperUnavailable)
    }

    fn var_os(&self, _key: &str) -> Option<OsString> {
        None
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.home.clone())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(PathBuf::from("/bin/pv"))
    }

    fn stdout_is_terminal(&self) -> bool {
        self.terminal_width.get().is_some()
    }

    fn terminal_width(&self) -> Option<usize> {
        self.terminal_width.get()
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }

    fn inspect_system_resolver(
        &self,
        _expected: Option<&platform::ResolverConfig>,
    ) -> ResolverFileState {
        self.resolver.borrow().clone()
    }
}

#[test]
fn dns_status_reports_the_dns_port_and_the_system_resolver() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let environment = TestEnvironment::new(&home);
    let set_resolver = |state| *environment.resolver.borrow_mut() = state;

    let unassigned = run_pv(&["dns:status"], &environment)?;
    Database::open(&PvPaths::for_home(home.clone()))?
        .assign_port(PortRequest::dns(35353, 35353, 35353), |_port| true)?;
    let missing = run_pv(&["dns:status"], &environment)?;
    set_resolver(ResolverFileState::Current {
        path: SYSTEM_RESOLVER_TEST_PATH.into(),
        port: 35353,
    });
    let current = run_pv(&["dns:status"], &environment)?;
    set_resolver(ResolverFileState::Stale {
        path: SYSTEM_RESOLVER_TEST_PATH.into(),
        expected_port: Some(35353),
        actual_port: Some(45000),
    });
    let stale = run_pv(&["dns:status"], &environment)?;
    environment.terminal_width.set(Some(80));
    let stale_on_terminal = run_pv(&["dns:status", "--no-color"], &environment)?;
    environment.terminal_width.set(None);
    set_resolver(ResolverFileState::Conflict {
        path: SYSTEM_RESOLVER_TEST_PATH.into(),
    });
    let conflict = run_pv(&["dns:status"], &environment)?;

    for output in [&unassigned, &missing, &current, &stale, &conflict] {
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.is_empty());
    }
    assert_debug_snapshot!((
        unassigned,
        missing,
        current,
        stale,
        stale_on_terminal,
        conflict
    ));

    Ok(())
}
