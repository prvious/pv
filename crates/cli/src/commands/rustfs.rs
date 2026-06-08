use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use state::{Database, PortOwner, PvPaths, RuntimeObservedStatus, RuntimeSubject, StateError};

use crate::args::{RustfsInstallArgs, RustfsUninstallArgs};
use crate::commands::artifact_resource::{self, ArtifactResourceCommandSpec};
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

const NOT_RUNNING_MESSAGE: &str = "RustFS is not running for any linked Project";

pub(crate) fn install(
    args: RustfsInstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::install(spec(), args.track.as_deref(), environment, stdout)
}

pub(crate) fn update(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::update(spec(), environment, stdout)
}

pub(crate) fn uninstall(
    args: RustfsUninstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::uninstall(
        spec(),
        &args.track,
        args.prune,
        args.force,
        environment,
        stdout,
    )
}

pub(crate) fn list(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    artifact_resource::list(spec(), environment, stdout)
}

pub(crate) fn open(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let mut output = Output::new(stdout, OutputMode::plain());

    let Some(url) = running_console_url(&database)? else {
        output.line(NOT_RUNNING_MESSAGE)?;
        return Ok(ExitCode::SUCCESS);
    };

    environment.open_url(&url)?;
    output.line(&format!("Opened RustFS console at {url}"))?;

    Ok(ExitCode::SUCCESS)
}

fn spec() -> ArtifactResourceCommandSpec {
    ArtifactResourceCommandSpec {
        resource_name: "rustfs",
        display_name: "RustFS",
        adapter: resources::rustfs_adapter,
    }
}

fn running_console_url(database: &Database) -> Result<Option<String>, ExecuteError> {
    for state in database.runtime_observed_states()? {
        let RuntimeSubject::Resource { name, track } = state.subject else {
            continue;
        };
        if name != "rustfs" || state.status != RuntimeObservedStatus::Running {
            continue;
        }
        if let Some(port) = console_port(database, &track)? {
            return Ok(Some(format!("http://127.0.0.1:{port}")));
        }
    }

    Ok(None)
}

fn console_port(database: &Database, track: &str) -> Result<Option<u16>, ExecuteError> {
    Ok(database
        .assigned_ports()?
        .into_iter()
        .find_map(|assignment| {
            if assignment.owner
                == (PortOwner::Resource {
                    name: "rustfs".to_string(),
                    track: track.to_string(),
                    port: "console".to_string(),
                })
            {
                return Some(assignment.port);
            }

            None
        }))
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}
