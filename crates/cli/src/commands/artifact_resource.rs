use std::collections::BTreeMap;
use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use resources::{
    ManagedResourceCommands, ManagedResourceTrack, ManagedResourceUninstallOptions,
    ResourceHttpClient, ResourceKind, ResourceName, TargetPlatform, TrackName, TrackSelector,
    UreqResourceHttpClient,
};
use serde::Serialize;
use state::{PortAssignment, PortOwner, PvPaths, RuntimeObservedStatus, StateError};

use crate::args::ListArgs;
use crate::environment::{Environment, artifact_manifest_url};
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

const RECONCILE_KIND: &str = "reconcile";
const SYSTEM_SCOPE: &str = "system";

pub(crate) struct ArtifactResourceCommandSpec {
    pub resource_name: &'static str,
    pub display_name: &'static str,
    pub adapter: fn() -> resources::Result<resources::RuntimeArtifactAdapter>,
}

pub(crate) fn install(
    spec: ArtifactResourceCommandSpec,
    track: Option<&str>,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let selector = match track {
        Some(track) => TrackSelector::parse(track)?,
        None => TrackSelector::Latest,
    };
    let adapter = (spec.adapter)()?;
    let commands = resource_commands(&paths, environment);
    let installed = with_resource_http_client(environment, |client| {
        commands.install(&adapter, selector, client)
    })?;
    let mut output = Output::new(stdout, OutputMode::plain());

    super::write_revoked_latest_warning(&installed, &mut output)?;
    output.line(&format!(
        "Installed {} track {}",
        spec.display_name,
        installed.track()
    ))?;
    request_system_reconciliation(&paths, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn update(
    spec: ArtifactResourceCommandSpec,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let adapter = (spec.adapter)()?;
    let commands = resource_commands(&paths, environment);
    let updated =
        with_resource_http_client(environment, |client| commands.update(&adapter, client))?;
    let mut output = Output::new(stdout, OutputMode::plain());

    super::write_revoked_latest_warnings(updated.installs(), &mut output)?;
    output.line(&format!(
        "Updated {} {} track(s)",
        updated.installs().len(),
        spec.display_name
    ))?;
    request_system_reconciliation(&paths, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn uninstall(
    spec: ArtifactResourceCommandSpec,
    track: &str,
    prune: bool,
    force: bool,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let resource_name = ResourceName::new(spec.resource_name)?;
    let track = TrackName::new(track)?;
    if prune && !force && !confirm_prune(&spec, track.as_str(), environment, stdout)? {
        return Ok(ExitCode::SUCCESS);
    }
    let options = ManagedResourceUninstallOptions::new()
        .prune(prune)
        .force(force);
    let commands = resource_commands(&paths, environment);
    let removal = commands.uninstall(&resource_name, &track, options)?;
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line(&format!(
        "Queued removal for {} track {}",
        spec.display_name,
        removal.track()
    ))?;
    request_system_reconciliation(&paths, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn list(
    spec: ArtifactResourceCommandSpec,
    args: ListArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let resource_name = ResourceName::new(spec.resource_name)?;
    let commands = resource_commands(&paths, environment);
    let tracks = commands.list(Some(&resource_name))?;
    let descriptor = resources::registry::resolve_canonical(spec.resource_name)?;

    if args.json {
        let tracks = if descriptor.kind() == ResourceKind::BackingService {
            backing_resource_json_tracks(&paths, spec.resource_name, &tracks)?
        } else {
            resource_json_tracks(&tracks)
        };
        serde_json::to_writer(&mut *stdout, &ResourceListOutput { tracks })?;
        writeln!(stdout)?;

        return Ok(ExitCode::SUCCESS);
    }

    let mut output = Output::new(stdout, OutputMode::plain());

    if tracks.is_empty() {
        output.line(&format!("No {} tracks installed", spec.display_name))?;
        return Ok(ExitCode::SUCCESS);
    }

    if descriptor.kind() == ResourceKind::BackingService {
        write_backing_resource_list(&paths, spec.resource_name, &tracks, &mut output)?;
        return Ok(ExitCode::SUCCESS);
    }

    output.line("Track  Projects  Version  Path")?;
    for track in tracks {
        output.line(&format!(
            "{}  {}  {}  {}",
            track.track(),
            track.usage_count(),
            track.installed_version(),
            track.current_artifact_path()
        ))?;
    }

    Ok(ExitCode::SUCCESS)
}

fn confirm_prune(
    spec: &ArtifactResourceCommandSpec,
    track: &str,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<bool, ExecuteError> {
    let mut output = Output::new(stdout, OutputMode::plain());

    if !environment.stdin_is_terminal() {
        output.line(&format!(
            "Refusing to prune {} track {track} without an interactive confirmation.",
            spec.display_name
        ))?;
        output.line(&format!(
            "Rerun with `pv {}:uninstall {track} --prune --force` to prune non-interactively.",
            spec.resource_name
        ))?;

        return Ok(false);
    }

    output.line(&format!(
        "Prune PV-owned data for {} track {track}?",
        spec.display_name
    ))?;
    output.line("Type `yes` to continue.")?;

    if environment.read_line()?.trim() == "yes" {
        return Ok(true);
    }

    output.line("Prune cancelled.")?;

    Ok(false)
}

fn write_backing_resource_list(
    paths: &PvPaths,
    resource_name: &str,
    tracks: &[ManagedResourceTrack],
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    let observation = backing_resource_observation(paths, resource_name)?;

    output.line("Track  Status  Ports  Projects  Version  Path")?;
    for track in tracks {
        let track_name = track.track().as_str();
        let status = observation.runtime_statuses.get(track_name).copied();
        let ports = if status == Some(RuntimeObservedStatus::Running) {
            let ports = backing_resource_ports(&observation.assignments, resource_name, track_name);
            format_backing_resource_ports(&ports)
        } else {
            "-".to_string()
        };

        output.line(&format!(
            "{}  {}  {}  {}  {}  {}",
            track.track(),
            runtime_status_label(status),
            ports,
            track.usage_count(),
            track.installed_version(),
            track.current_artifact_path()
        ))?;
    }

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

fn resource_commands(paths: &PvPaths, environment: &impl Environment) -> ManagedResourceCommands {
    ManagedResourceCommands::new(
        paths.clone(),
        artifact_manifest_url(environment),
        target_platform(environment),
    )
}

fn target_platform(environment: &impl Environment) -> TargetPlatform {
    environment
        .target_platform()
        .unwrap_or_else(current_target_platform)
}

fn current_target_platform() -> TargetPlatform {
    if cfg!(target_arch = "aarch64") {
        TargetPlatform::DarwinArm64
    } else {
        TargetPlatform::DarwinAmd64
    }
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

fn request_system_reconciliation(
    paths: &PvPaths,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    match daemon::submit_job_blocking(paths.clone(), RECONCILE_KIND, SYSTEM_SCOPE) {
        Ok(job) => output.line(&format!("System reconciliation requested: {}", job.id))?,
        Err(daemon::DaemonError::Io(error)) if daemon_is_unavailable(&error) => {
            write_daemon_unavailable_warning(output)?
        }
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

fn write_daemon_unavailable_warning(
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    output.line(
        "warning: PV daemon is not running; reconciliation will run after `pv setup` starts it",
    )?;

    Ok(())
}

fn daemon_is_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::io;
    use std::path::PathBuf;

    use camino::Utf8Path;
    use camino_tempfile::tempdir;
    use insta::{Settings, assert_debug_snapshot};
    use state::{
        Database, LinkProjectInput, ManagedResourceDesiredState, ManagedResourceTrackRecord,
        PortRequest, ProjectManagedResourceInput, PvPaths, RuntimeObservedStatus, RuntimeSubject,
    };

    use super::*;

    #[derive(Debug)]
    struct TestEnvironment {
        home: PathBuf,
        stdin_is_terminal: bool,
        lines: RefCell<VecDeque<String>>,
    }

    impl TestEnvironment {
        fn new(home: &Utf8Path) -> Self {
            Self {
                home: home.as_std_path().to_path_buf(),
                stdin_is_terminal: false,
                lines: RefCell::new(VecDeque::new()),
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
            self.stdin_is_terminal
        }

        fn read_line(&self) -> io::Result<String> {
            Ok(self.lines.borrow_mut().pop_front().unwrap_or_default())
        }

        fn open_url(&self, _url: &str) -> io::Result<()> {
            Ok(())
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

        let exit_code = uninstall(redis_spec(), "7.2", true, false, &environment, &mut stdout)?;
        let record = database.managed_resource_track("redis", "7.2")?;

        assert_eq!(exit_code, ExitCode::SUCCESS);
        assert_eq!(record.desired_state, ManagedResourceDesiredState::Installed);
        assert!(!record.removal_prune);
        assert!(!record.removal_force);
        with_tempdir_filters(tempdir.path(), || {
            assert_debug_snapshot!((
                RunOutput::from_stdout(exit_code, stdout)?,
                resource_record_snapshot(&record, tempdir.path())?,
            ));
            Ok(())
        })?;

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

        let exit_code = list(
            mailpit_spec(),
            ListArgs { json: false },
            &environment,
            &mut stdout,
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

    #[derive(Debug)]
    #[expect(
        dead_code,
        reason = "snapshot-only structure is read through derived Debug"
    )]
    struct ResourceRecordSnapshot {
        resource_name: String,
        track: String,
        desired_state: String,
        installed_version: Option<String>,
        current_artifact_path: Option<String>,
        usage_count: i64,
        removal_prune: bool,
        removal_force: bool,
    }

    fn resource_record_snapshot(
        record: &ManagedResourceTrackRecord,
        root: &Utf8Path,
    ) -> anyhow::Result<ResourceRecordSnapshot> {
        Ok(ResourceRecordSnapshot {
            resource_name: record.resource_name.clone(),
            track: record.track.clone(),
            desired_state: format!("{:?}", record.desired_state),
            installed_version: record.installed_version.clone(),
            current_artifact_path: record
                .current_artifact_path
                .as_ref()
                .map(|path| path.strip_prefix(root).map(Utf8Path::to_string))
                .transpose()?,
            usage_count: record.usage_count,
            removal_prune: record.removal_prune,
            removal_force: record.removal_force,
        })
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
