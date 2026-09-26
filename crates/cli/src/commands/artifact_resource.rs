use std::collections::BTreeMap;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use resources::{
    ManagedResourceCommands, ManagedResourceTrack, ManagedResourceUninstallOptions,
    ResourceHttpClient, ResourceKind, ResourceName, TrackName, TrackSelector,
    UreqResourceHttpClient,
};
use serde::Serialize;
use state::{PortAssignment, PortOwner, PvPaths, RuntimeObservedStatus, StateError};

use crate::args::ListArgs;
use crate::environment::{Environment, artifact_manifest_url};
use crate::error::{CliError, ExecuteError};
use crate::output::{Line, Output, Streams, Table};
use crate::progress::DownloadProgressRenderer;
use crate::prompt;

pub(crate) struct ArtifactResourceCommandSpec {
    pub resource_name: &'static str,
    pub display_name: &'static str,
    pub adapter: fn() -> resources::Result<resources::RuntimeArtifactAdapter>,
}

pub(crate) fn install(
    spec: ArtifactResourceCommandSpec,
    track: Option<&str>,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let selector = match track {
        Some(track) => TrackSelector::parse(track)?,
        None => TrackSelector::Latest,
    };
    let adapter = (spec.adapter)()?;
    let commands = resource_commands(&paths, environment)?;
    let jobs_lock = super::acquire_jobs_lock(&paths)?;
    let progress = DownloadProgressRenderer::new(&streams.err);
    let installed = with_resource_http_client(environment, |client| {
        commands.install_with_progress(&adapter, selector, client, &progress)
    })?;
    drop(progress);
    drop(jobs_lock);
    let output = &mut streams.out;

    super::write_revoked_latest_warning(&installed, &mut streams.err)?;
    output.success(Line::field(
        &format!("Installed {} track ", spec.display_name),
        installed.track(),
    ))?;
    super::request_system_reconciliation(&paths, streams)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn update(
    spec: ArtifactResourceCommandSpec,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let adapter = (spec.adapter)()?;
    let commands = resource_commands(&paths, environment)?;
    let jobs_lock = super::acquire_jobs_lock(&paths)?;
    let progress = DownloadProgressRenderer::new(&streams.err);
    let updated = with_resource_http_client(environment, |client| {
        commands.update_with_progress(&adapter, client, &progress)
    })?;
    drop(progress);
    drop(jobs_lock);
    let output = &mut streams.out;

    super::write_revoked_latest_warnings(updated.installs(), &mut streams.err)?;
    super::write_updated(
        output,
        updated.installs().len(),
        &format!("{} track(s)", spec.display_name),
    )?;
    super::request_system_reconciliation(&paths, streams)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn uninstall(
    spec: ArtifactResourceCommandSpec,
    track: &str,
    prune: bool,
    force: bool,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let resource_name = ResourceName::new(spec.resource_name)?;
    let track = TrackName::new(track)?;
    let commands = resource_commands(&paths, environment)?;
    if prune && !force {
        let refusal = CliError::ResourcePruneRequiresTerminal {
            display_name: spec.display_name,
            resource_name: spec.resource_name,
            track: track.to_string(),
        };
        let message = format!(
            "Prune PV-owned data for {} track {track}?",
            spec.display_name
        );
        if !prompt::confirm_or(environment, streams, refusal, &message, false)? {
            streams.out.note("Prune cancelled.")?;
            return Ok(ExitCode::SUCCESS);
        }
    }
    let options = ManagedResourceUninstallOptions::new()
        .prune(prune)
        .force(force);
    let removal = commands.uninstall(&resource_name, &track, options)?;
    let output = &mut streams.out;

    output.success(Line::field(
        &format!("Queued removal for {} track ", spec.display_name),
        removal.track(),
    ))?;
    super::request_system_reconciliation(&paths, streams)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn list(
    spec: ArtifactResourceCommandSpec,
    args: ListArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let resource_name = ResourceName::new(spec.resource_name)?;
    let commands = resource_commands(&paths, environment)?;
    let tracks = commands.list(Some(&resource_name))?;
    let descriptor = resources::registry::resolve_canonical(spec.resource_name)?;

    if args.json {
        let tracks = if descriptor.kind() == ResourceKind::BackingService {
            backing_resource_json_tracks(&paths, spec.resource_name, &tracks)?
        } else {
            resource_json_tracks(&tracks)
        };
        streams.out.json(&ResourceListOutput { tracks })?;

        return Ok(ExitCode::SUCCESS);
    }

    let output = &mut streams.out;

    if tracks.is_empty() {
        output.note(format!("No {} tracks installed", spec.display_name))?;
        return Ok(ExitCode::SUCCESS);
    }

    if descriptor.kind() == ResourceKind::BackingService {
        write_backing_resource_list(&paths, spec.resource_name, &tracks, output)?;
        return Ok(ExitCode::SUCCESS);
    }

    let mut table = Table::new(&["Track", "Projects", "Version", "Path"]);
    for track in tracks {
        table.row(vec![
            Line::from(track.track().as_str()),
            Line::from(track.usage_count().to_string()),
            Line::default().value(track.installed_version().as_str()),
            Line::from(track.current_artifact_path().as_str()),
        ]);
    }
    output.table(&table)?;

    Ok(ExitCode::SUCCESS)
}

fn write_backing_resource_list(
    paths: &PvPaths,
    resource_name: &str,
    tracks: &[ManagedResourceTrack],
    output: &mut Output<'_>,
) -> Result<(), ExecuteError> {
    let observation = backing_resource_observation(paths, resource_name)?;

    let mut table = Table::new(&["Track", "Status", "Ports", "Projects", "Version", "Path"]);
    for track in tracks {
        let track_name = track.track().as_str();
        let status = observation.runtime_statuses.get(track_name).copied();
        let ports = if status == Some(RuntimeObservedStatus::Running) {
            let ports = backing_resource_ports(&observation.assignments, resource_name, track_name);
            format_backing_resource_ports(&ports)
        } else {
            "-".to_string()
        };

        table.row(vec![
            Line::from(track.track().as_str()),
            Line::marked(super::runtime_mark(status), runtime_status_label(status)),
            Line::default().value(ports),
            Line::from(track.usage_count().to_string()),
            Line::default().value(track.installed_version().as_str()),
            Line::from(track.current_artifact_path().as_str()),
        ]);
    }
    output.table(&table)?;

    Ok(())
}

#[derive(Serialize)]
struct ResourceListOutput {
    tracks: Vec<ResourceListTrack>,
}

#[derive(Serialize)]
struct ResourceListTrack {
    resource: String,
    track: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'static str>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    ports: BTreeMap<String, u16>,
    projects: i64,
    version: String,
    path: String,
}

struct BackingResourceObservation {
    runtime_statuses: BTreeMap<String, RuntimeObservedStatus>,
    assignments: Vec<PortAssignment>,
}

fn backing_resource_json_tracks(
    paths: &PvPaths,
    resource_name: &str,
    tracks: &[ManagedResourceTrack],
) -> Result<Vec<ResourceListTrack>, ExecuteError> {
    let observation = backing_resource_observation(paths, resource_name)?;

    Ok(tracks
        .iter()
        .map(|track| {
            let track_name = track.track().as_str();
            let status = observation.runtime_statuses.get(track_name).copied();
            let ports = if status == Some(RuntimeObservedStatus::Running) {
                backing_resource_ports(&observation.assignments, resource_name, track_name)
            } else {
                BTreeMap::new()
            };

            resource_json_track(track, Some(runtime_status_label(status)), ports)
        })
        .collect())
}

fn resource_json_tracks(tracks: &[ManagedResourceTrack]) -> Vec<ResourceListTrack> {
    tracks
        .iter()
        .map(|track| resource_json_track(track, None, BTreeMap::new()))
        .collect()
}

fn resource_json_track(
    track: &ManagedResourceTrack,
    status: Option<&'static str>,
    ports: BTreeMap<String, u16>,
) -> ResourceListTrack {
    ResourceListTrack {
        resource: track.resource_name().as_str().to_string(),
        track: track.track().as_str().to_string(),
        status,
        ports,
        projects: track.usage_count(),
        version: track.installed_version().as_str().to_string(),
        path: track.current_artifact_path().to_string(),
    }
}

fn backing_resource_observation(
    paths: &PvPaths,
    resource_name: &str,
) -> Result<BackingResourceObservation, ExecuteError> {
    let database = state::Database::open(paths)?;
    let runtime_statuses = database
        .runtime_observed_states()?
        .into_iter()
        .filter_map(|state| match state.subject {
            state::RuntimeSubject::Resource { name, track } if name == resource_name => {
                Some((track, state.status))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let assignments = database.assigned_ports()?;

    Ok(BackingResourceObservation {
        runtime_statuses,
        assignments,
    })
}

fn backing_resource_ports(
    assignments: &[PortAssignment],
    resource_name: &str,
    track: &str,
) -> BTreeMap<String, u16> {
    assignments
        .iter()
        .filter_map(|assignment| match &assignment.owner {
            PortOwner::Resource {
                name,
                track: owner_track,
                port,
            } if name == resource_name && owner_track == track => {
                Some((port.clone(), assignment.port))
            }
            _ => None,
        })
        .collect()
}

fn format_backing_resource_ports(ports: &BTreeMap<String, u16>) -> String {
    if ports.is_empty() {
        return "-".to_string();
    }

    ports
        .iter()
        .map(|(name, port)| format!("{name}={port}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn runtime_status_label(status: Option<RuntimeObservedStatus>) -> &'static str {
    match status {
        Some(RuntimeObservedStatus::Pending) => "pending",
        Some(RuntimeObservedStatus::Running) => "running",
        Some(RuntimeObservedStatus::Degraded) => "degraded",
        Some(RuntimeObservedStatus::Failed) => "failed",
        Some(RuntimeObservedStatus::Stopped) => "stopped",
        None => "not-running",
    }
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

fn resource_commands(
    paths: &PvPaths,
    environment: &impl Environment,
) -> Result<ManagedResourceCommands, ExecuteError> {
    Ok(ManagedResourceCommands::new(
        paths.clone(),
        artifact_manifest_url(environment),
        environment.resolve_target_platform()?,
    ))
}

fn with_resource_http_client<T>(
    environment: &impl Environment,
    operation: impl FnOnce(&dyn ResourceHttpClient) -> Result<T, resources::ManagedResourceCommandError>,
) -> Result<T, ExecuteError> {
    if let Some(client) = environment.resource_http_client() {
        return Ok(operation(client)?);
    }

    let client = UreqResourceHttpClient::default();
    Ok(operation(&client)?)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io;
    use std::path::PathBuf;

    use camino::Utf8Path;
    use camino_tempfile::tempdir;
    use insta::{Settings, assert_debug_snapshot};
    use state::{
        Database, LinkProjectInput, ManagedResourceDesiredState, PortRequest,
        ProjectManagedResourceInput, PvPaths, RuntimeObservedStatus, RuntimeSubject,
    };

    use super::*;
    use crate::output::Presentation;

    #[derive(Debug)]
    struct TestEnvironment {
        home: PathBuf,
    }

    impl TestEnvironment {
        fn new(home: &Utf8Path) -> Self {
            Self {
                home: home.as_std_path().to_path_buf(),
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
            Ok(self.home.clone())
        }

        fn current_exe(&self) -> io::Result<PathBuf> {
            Ok(PathBuf::from("/bin/pv"))
        }

        fn stdin_is_terminal(&self) -> bool {
            false
        }

        fn open_url(&self, _url: &str) -> io::Result<()> {
            Ok(())
        }

        fn target_platform(&self) -> Option<resources::TargetPlatform> {
            Some(resources::TargetPlatform::DarwinArm64)
        }
    }

    #[test]
    fn uninstall_prune_refuses_noninteractive_without_force() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let environment = TestEnvironment::new(paths.home());
        let mut database = Database::open(&paths)?;
        let release = paths
            .resources()
            .join("redis")
            .join("7.2")
            .join("releases")
            .join("7.2.5-pv1");

        database.record_managed_resource_track_installed("redis", "7.2", "7.2.5-pv1", &release)?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let result = uninstall(
            redis_spec(),
            "7.2",
            true,
            false,
            &environment,
            &mut Streams::new(&mut stdout, &mut stderr, Presentation::plain()),
        );
        let record = database.managed_resource_track("redis", "7.2")?;

        assert!(matches!(
            result,
            Err(ExecuteError::User(
                CliError::ResourcePruneRequiresTerminal { .. }
            ))
        ));
        assert!(stdout.is_empty());
        assert_eq!(record.desired_state, ManagedResourceDesiredState::Installed);
        assert!(!record.removal_prune);
        assert!(!record.removal_force);

        Ok(())
    }

    #[test]
    fn backing_resource_list_reports_running_state_ports_and_usage() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let environment = TestEnvironment::new(paths.home());
        let mut database = Database::open(&paths)?;
        let release = paths
            .resources()
            .join("mailpit")
            .join("1")
            .join("releases")
            .join("1.20.0-pv1");

        database.record_managed_resource_track_installed("mailpit", "1", "1.20.0-pv1", &release)?;
        let project = database
            .link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path.clone(),
                primary_hostname: "acme.test".to_string(),
                config_path: project_path.join("pv.yml"),
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?
            .project;
        database.replace_project_managed_resources(
            &project.id,
            &[ProjectManagedResourceInput {
                resource_name: "mailpit".to_string(),
                track: "1".to_string(),
            }],
        )?;
        database.assign_port(
            PortRequest::resource_port("mailpit", "1", "smtp", 1025, 45000, 48999),
            |_| true,
        )?;
        database.assign_port(
            PortRequest::resource_port("mailpit", "1", "dashboard", 8025, 45000, 48999),
            |_| true,
        )?;
        database.record_runtime_observed_snapshot(
            RuntimeSubject::Resource {
                name: "mailpit".to_string(),
                track: "1".to_string(),
            },
            RuntimeObservedStatus::Running,
            Some("Managed Resource runtime is ready"),
        )?;
        let mut stdout = Vec::new();

        let mut stderr = Vec::new();
        let exit_code = list(
            mailpit_spec(),
            ListArgs { json: false },
            &environment,
            &mut Streams::new(&mut stdout, &mut stderr, Presentation::plain()),
        )?;

        assert_eq!(exit_code, ExitCode::SUCCESS);
        with_tempdir_filters(tempdir.path(), || {
            assert_debug_snapshot!(RunOutput::from_stdout(exit_code, stdout)?);
            Ok(())
        })?;

        Ok(())
    }

    #[derive(Debug)]
    #[expect(
        dead_code,
        reason = "snapshot-only structure is read through derived Debug"
    )]
    struct RunOutput {
        exit_code: ExitCode,
        stdout: String,
    }

    impl RunOutput {
        fn from_stdout(exit_code: ExitCode, stdout: Vec<u8>) -> anyhow::Result<Self> {
            Ok(Self {
                exit_code,
                stdout: String::from_utf8(stdout)?,
            })
        }
    }

    fn redis_spec() -> ArtifactResourceCommandSpec {
        ArtifactResourceCommandSpec {
            resource_name: "redis",
            display_name: "Redis",
            adapter: resources::composer_adapter,
        }
    }

    fn mailpit_spec() -> ArtifactResourceCommandSpec {
        ArtifactResourceCommandSpec {
            resource_name: "mailpit",
            display_name: "Mailpit",
            adapter: resources::composer_adapter,
        }
    }

    fn with_tempdir_filters(
        tempdir: &Utf8Path,
        f: impl FnOnce() -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let mut settings = Settings::clone_current();
        settings.add_filter(&regex_literal(tempdir.as_str()), "<tempdir>");
        settings.bind(f)
    }

    fn regex_literal(input: &str) -> String {
        let mut escaped = String::with_capacity(input.len());

        for character in input.chars() {
            if matches!(
                character,
                '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
            ) {
                escaped.push('\\');
            }
            escaped.push(character);
        }

        escaped
    }
}
