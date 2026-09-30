use std::io;

use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::events::{Event, read_events};
use crate::{Persona, Scenario, events_path, lifeline, scenario_path};

/// A fake installed at a runtime's executable path.
#[derive(Clone, Debug)]
pub struct InstalledFake {
    executable: Utf8PathBuf,
}

impl InstalledFake {
    pub fn executable(&self) -> &Utf8Path {
        &self.executable
    }

    /// Events the fake has recorded so far; empty until it first runs.
    pub fn events(&self) -> Result<Vec<Event>> {
        read_events(&events_path(&self.executable))
    }
}

/// Installs the daemon's `pv-fake` example at `executable`, tied to this test process's lifeline.
pub fn install(executable: &Utf8Path, persona: Persona) -> Result<InstalledFake> {
    let scenario = Scenario {
        persona,
        lifeline_fd: Some(lifeline::test_process_read_fd()?),
    };

    install_with(&binary()?, executable, &scenario)
}

/// Installs `binary` at `executable` and writes `scenario` next to it.
pub fn install_with(
    binary: &Utf8Path,
    executable: &Utf8Path,
    scenario: &Scenario,
) -> Result<InstalledFake> {
    if let Some(parent) = executable.parent() {
        state::fs::ensure_user_dir(parent)?;
    }
    link_or_copy(binary, executable)?;
    state::fs::write_sensitive_file(
        &scenario_path(executable),
        &serde_json::to_string_pretty(scenario)?,
    )?;

    Ok(InstalledFake {
        executable: executable.to_owned(),
    })
}

/// The daemon's `pv-fake` example, which Cargo builds along with the daemon's tests.
#[expect(
    clippy::disallowed_methods,
    reason = "pv-fake finds the example target next to the running test executable"
)]
pub fn binary() -> Result<Utf8PathBuf> {
    let test_executable = Utf8PathBuf::try_from(std::env::current_exe()?)?;
    // Test executables run from target/<profile>/deps; examples live in target/<profile>/examples.
    let Some(profile_dir) = test_executable.parent().and_then(Utf8Path::parent) else {
        bail!("test executable {test_executable} is not inside a Cargo target directory");
    };
    let binary = profile_dir.join("examples/pv-fake");
    if !state::fs::path_is_file(&binary)? {
        bail!(
            "pv-fake binary {binary} is missing; `cargo nextest run -p daemon` builds the daemon's \
             examples, `cargo test --lib` does not"
        );
    }

    Ok(binary)
}

/// Hard-links the fake into place, copying across filesystems.
///
/// A hard link is a regular file, so PV's artifact validation, which rejects symlinked
/// executables, accepts it. The process executable is the path the fake was started from, so the
/// supervisor's direct identity check applies, as it does to real runtime binaries.
#[expect(
    clippy::disallowed_methods,
    reason = "pv-fake links its own binary into fake artifact directories"
)]
fn link_or_copy(binary: &Utf8Path, executable: &Utf8Path) -> Result<()> {
    match std::fs::hard_link(binary, executable) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
            state::fs::copy_file_atomically(binary, executable)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
