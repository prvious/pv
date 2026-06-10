use std::collections::BTreeSet;
use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use state::{Database, PvPaths, RuntimeObservedStatus, RuntimeSubject, StateError};

use crate::args::{ListArgs, MailpitInstallArgs, MailpitUninstallArgs};
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

const NOT_RUNNING_MESSAGE: &str = "Mailpit is not running for any linked Project";

const SPEC: super::artifact_resource::ArtifactResourceCommandSpec =
    super::artifact_resource::ArtifactResourceCommandSpec {
        resource_name: "mailpit",
        display_name: "Mailpit",
        adapter: resources::mailpit_adapter,
    };

pub(crate) fn install(
    args: MailpitInstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    super::artifact_resource::install(SPEC, args.track.as_deref(), environment, stdout)
}

pub(crate) fn update(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    super::artifact_resource::update(SPEC, environment, stdout)
}

pub(crate) fn uninstall(
    args: MailpitUninstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    super::artifact_resource::uninstall(
        SPEC,
        &args.track,
        args.prune,
        args.force,
        environment,
        stdout,
    )
}

pub(crate) fn list(
    args: ListArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    super::artifact_resource::list(SPEC, args, environment, stdout)
}

pub(crate) fn open(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let Some(url) = running_dashboard_url(&database)? else {
        let mut output = Output::new(stdout, OutputMode::plain());
        output.line(NOT_RUNNING_MESSAGE)?;

        return Ok(ExitCode::SUCCESS);
    };

    environment.open_url(&url)?;

    Ok(ExitCode::SUCCESS)
}

fn running_dashboard_url(database: &Database) -> Result<Option<String>, ExecuteError> {
    let running_tracks = database
        .runtime_observed_states()?
        .into_iter()
        .filter_map(|state| match (state.subject, state.status) {
            (RuntimeSubject::Resource { name, track }, RuntimeObservedStatus::Running)
                if name == SPEC.resource_name =>
            {
                Some(track)
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();

    for track in database.managed_resource_tracks()? {
        if track.resource_name != SPEC.resource_name
            || track.usage_count == 0
            || !running_tracks.contains(&track.track)
        {
            continue;
        }
        if let Some(url) = track.env.get("dashboard_url") {
            return Ok(Some(url.clone()));
        }
    }

    Ok(None)
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}
