use std::collections::BTreeMap;
use std::io;
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

    super::write_revoked_latest_warning(&installed, &mut streams.err);
    let mut summary = Line::field(
        &format!("Installed {} track ", spec.display_name),
        installed.track(),
    );
    if !output.surface().decorated() {
        summary = summary
            .text(" (")
            .value(installed.artifact_version().as_str())
            .text(")");
    }
    output.success(summary)?;
    if output.surface().decorated() {
        output.detail(Line::from("·  ").value(installed.artifact_version().as_str()))?;
    }
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

    super::write_revoked_latest_warnings(updated.installs(), &mut streams.err);
    if output.surface().decorated() && updated.installs().is_empty() {
        output.success(format!("{} is up to date.", spec.display_name))?;
    } else if output.surface().decorated() && updated.installs().len() == 1 {
        output.success(
            Line::from(format!("{} updated · ", spec.display_name))
                .value(updated.installs()[0].artifact_version().as_str()),
        )?;
    } else {
        super::write_updated(
            output,
            updated.installs().len(),
            &format!("{} track(s)", spec.display_name),
        )?;
    }
    if !output.surface().decorated() || updated.installs().len() > 1 {
        for install in updated.installs() {
            output.detail(
                Line::from(format!("{}  ", install.track()))
                    .value(install.artifact_version().as_str()),
            )?;
        }
    }
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
            super::write_prune_cancelled(&mut streams.out)?;
            return Ok(ExitCode::SUCCESS);
        }
    }
    let options = ManagedResourceUninstallOptions::new()
        .prune(prune)
        .force(force);
    let removal = commands.uninstall(&resource_name, &track, options)?;
    let output = &mut streams.out;

    let summary = if output.surface().decorated() {
        format!("Removal requested for {} track ", spec.display_name)
    } else {
        format!("Queued removal for {} track ", spec.display_name)
    };
    let displayed = (|| -> io::Result<()> {
        output.success(Line::field(&summary, removal.track()))?;
        let prefix = if output.surface().decorated() {
            "·  "
        } else {
            ""
        };
        if prune {
            output.detail(format!("{prefix}data will be pruned"))?;
        } else {
            let data_path = paths.resource_data_dir(spec.resource_name, removal.track().as_str());
            if data_path.exists() {
                output.detail(Line::field(
                    &format!("{prefix}data preserved under "),
                    data_path,
                ))?;
            }
        }
        Ok(())
    })();
    super::request_system_reconciliation(&paths, streams)?;
    displayed?;

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
        if output.surface().decorated() {
            output.hint("install one", &format!("pv {}:install", spec.resource_name))?;
        }
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

    let decorated = output.surface().decorated();
    let mut table = if decorated {
        Table::new(&["Track", "Status", "Ports", "Projects", "Version"])
    } else {
        Table::new(&["Track", "Status", "Ports", "Projects", "Version", "Path"])
    };
    for track in tracks {
        let track_name = track.track().as_str();
        let status = observation.runtime_statuses.get(track_name).copied();
        let ports = if status == Some(RuntimeObservedStatus::Running) {
            let ports = backing_resource_ports(&observation.assignments, resource_name, track_name);
            format_backing_resource_ports(&ports)
        } else {
            "-".to_string()
        };

        let mut cells = vec![
            Line::from(track.track().as_str()),
            Line::marked(super::runtime_mark(status), runtime_status_label(status)),
            Line::default().value(ports),
            Line::from(track.usage_count().to_string()),
            Line::default().value(track.installed_version().as_str()),
        ];
        if !decorated {
            cells.push(Line::from(track.current_artifact_path().as_str()));
        }
        table.row(cells);
        if decorated {
            table.detail(Line::default().value(track.current_artifact_path().as_str()));
        }
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
