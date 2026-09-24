use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use resources::{ArtifactManifestCache, ResourceName, TrackSelector};
use state::{Database, PvPaths, StateError};

use crate::args::LogsArgs;
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, Streams, Tone};

const MAX_LINE_COUNT: usize = 5000;

pub(crate) fn run(
    args: LogsArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let line_count = line_count(args.lines)?;
    let paths = pv_paths(environment)?;
    let selection = select_sources(&args, &paths)?;
    let color_enabled = streams.out.surface().color();

    write_initial_tail(
        &selection.sources,
        line_count,
        &selection.empty_message,
        color_enabled,
        &mut streams.out,
    )?;

    if args.follow {
        follow_sources(&selection.sources, color_enabled, streams.out.writer())?;
    }

    Ok(ExitCode::SUCCESS)
}

fn line_count(lines: i64) -> Result<usize, ExecuteError> {
    if lines < 0 {
        return Err(CliError::InvalidLogLineCount.into());
    }

    let lines = match usize::try_from(lines) {
        Ok(lines) => lines,
        Err(_) => MAX_LINE_COUNT,
    };

    Ok(lines.min(MAX_LINE_COUNT))
}

#[derive(Clone, Debug)]
struct LogSelection {
    sources: Vec<LogSource>,
    empty_message: String,
}

#[derive(Clone, Debug)]
struct LogSource {
    label: String,
    active_path: Utf8PathBuf,
}

fn select_sources(args: &LogsArgs, paths: &PvPaths) -> Result<LogSelection, ExecuteError> {
    if args.all {
        let mut sources = default_sources(paths);
        sources.extend(gateway_sources(paths));
        sources.extend(installed_worker_sources(paths)?);
        sources.extend(installed_resource_sources(paths)?);

        return Ok(LogSelection {
            sources,
            empty_message: "No PV log files found".to_string(),
        });
    }

    if args.gateway {
        return Ok(LogSelection {
            sources: gateway_sources(paths),
            empty_message: "No log files found for gateway".to_string(),
        });
    }

    if let Some(worker) = &args.worker {
        let worker = resolve_worker_track(paths, worker)?;

        return Ok(LogSelection {
            sources: vec![LogSource {
                label: format!("worker:{worker}"),
                active_path: paths.worker_log(&worker),
            }],
            empty_message: format!("No logs exist for PHP worker track {worker}"),
        });
    }

    if let Some(resource) = &args.resource {
        let descriptor = resources::registry::resolve(resource)?;
        let resource_name = descriptor.name();
        let track = resolve_resource_track(paths, resource_name, args.track.as_deref())?;

        return Ok(LogSelection {
            sources: vec![LogSource {
                label: format!("{resource_name}:{track}"),
                active_path: paths.resource_log(resource_name, &track),
            }],
            empty_message: format!("No logs exist for {resource_name} track {track}"),
        });
    }

    Ok(LogSelection {
        sources: default_sources(paths),
        empty_message: "No PV daemon logs found".to_string(),
    })
}

fn default_sources(paths: &PvPaths) -> Vec<LogSource> {
    vec![
        LogSource {
            label: "daemon".to_string(),
            active_path: paths.daemon_log(),
        },
        LogSource {
            label: "launchd:stdout".to_string(),
            active_path: paths.launchd_stdout_log(),
        },
        LogSource {
            label: "launchd:stderr".to_string(),
            active_path: paths.launchd_stderr_log(),
        },
    ]
}

fn gateway_sources(paths: &PvPaths) -> Vec<LogSource> {
    let access = paths.gateway_access_log();
    let error = paths.gateway_error_log();
    let supervisor = paths.gateway_supervisor_log();

    if state::fs::path_exists(&access)
        || state::fs::path_exists(&error)
        || state::fs::path_exists(&supervisor)
    {
        return vec![
            LogSource {
                label: "gateway:access".to_string(),
                active_path: access,
            },
            LogSource {
                label: "gateway:error".to_string(),
                active_path: error,
            },
            LogSource {
                label: "gateway:supervisor".to_string(),
                active_path: supervisor,
            },
        ];
    }

    vec![LogSource {
        label: "gateway".to_string(),
        active_path: paths.gateway_log(),
    }]
}

fn installed_worker_sources(paths: &PvPaths) -> Result<Vec<LogSource>, ExecuteError> {
    let Some(database) = Database::open_read_only(paths)? else {
        return Ok(Vec::new());
    };
    let mut tracks = BTreeSet::new();

    for state in database.runtime_observed_states()? {
        match state.subject {
            state::RuntimeSubject::PhpWorker { php_track } => {
                tracks.insert(php_track);
            }
            state::RuntimeSubject::PhpRuntimeWorker { php_runtime_key } => {
                tracks.insert(php_runtime_key);
            }
            state::RuntimeSubject::Gateway | state::RuntimeSubject::Resource { .. } => {}
        }
    }

    Ok(tracks
        .into_iter()
        .map(|track| LogSource {
            label: format!("worker:{track}"),
            active_path: paths.worker_log(&track),
        })
        .collect())
}

fn installed_resource_sources(paths: &PvPaths) -> Result<Vec<LogSource>, ExecuteError> {
    let Some(database) = Database::open_read_only(paths)? else {
        return Ok(Vec::new());
    };

    Ok(database
        .managed_resource_tracks()?
        .into_iter()
        .map(|track| LogSource {
            label: format!("{}:{}", track.resource_name, track.track),
            active_path: paths.resource_log(&track.resource_name, &track.track),
        })
        .collect())
}

fn resolve_resource_track(
    paths: &PvPaths,
    resource_name: &str,
    requested_track: Option<&str>,
) -> Result<String, ExecuteError> {
    if let Some(track) = requested_track {
        if TrackSelector::is_reserved_alias(track) {
            let resource = ResourceName::new(resource_name)?;
            let manifest = ArtifactManifestCache::new(paths.downloads()).load_cached()?;
            let track = manifest.resolve_track(&resource, TrackSelector::Latest)?;

            return Ok(track.as_str().to_string());
        }

        return Ok(track.to_string());
    }

    let Some(database) = Database::open_read_only(paths)? else {
        return Err(CliError::MissingLogResourceTrack {
            resource: resource_name.to_string(),
        }
        .into());
    };
    let tracks = database
        .managed_resource_tracks()?
        .into_iter()
        .filter(|track| track.resource_name == resource_name)
        .map(|track| track.track)
        .collect::<Vec<_>>();

    match tracks.as_slice() {
        [track] => Ok(track.clone()),
        [] => Err(CliError::MissingLogResourceTrack {
            resource: resource_name.to_string(),
        }
        .into()),
        tracks => Err(CliError::AmbiguousLogResourceTrack {
            resource: resource_name.to_string(),
            tracks: tracks.join(", "),
        }
        .into()),
    }
}

fn resolve_worker_track(paths: &PvPaths, worker: &str) -> Result<String, ExecuteError> {
    if !TrackSelector::is_reserved_alias(worker) {
        return Ok(worker.to_string());
    }

    if let Some(database) = Database::open_read_only(paths)?
        && let Some(track) = database.global_php_default_track()?
    {
        return Ok(track);
    }

    let php = ResourceName::new("php")?;
    let manifest = ArtifactManifestCache::new(paths.downloads()).load_cached()?;
    let track = manifest.resolve_track(&php, TrackSelector::Latest)?;

    Ok(track.as_str().to_string())
}

fn write_initial_tail(
    sources: &[LogSource],
    line_count: usize,
    empty_message: &str,
    color_enabled: bool,
    output: &mut Output<'_>,
) -> Result<(), ExecuteError> {
    let tails = sources
        .iter()
        .map(|source| source_tail(source, line_count))
        .collect::<Result<Vec<_>, _>>()?;
    let available_count = tails.iter().filter(|tail| tail.available).count();

    if available_count == 0 {
        output.note(empty_message)?;
        return Ok(());
    }

    let prefixed = available_count > 1;
    for tail in tails {
        let prefix = log_prefix(&tail.source.label, prefixed, color_enabled);
        for line in tail.lines {
            writeln!(output.writer(), "{prefix}{line}")?;
        }
    }

    Ok(())
}

#[derive(Debug)]
struct SourceTail<'source> {
    source: &'source LogSource,
    available: bool,
    lines: Vec<String>,
}

fn source_tail(source: &LogSource, line_count: usize) -> Result<SourceTail<'_>, ExecuteError> {
    let paths = log_paths_for_initial_tail(&source.active_path)?;
    let available = !paths.is_empty();
    let mut lines = Vec::new();

    if line_count == 0 {
        return Ok(SourceTail {
            source,
            available,
            lines,
        });
    }

    for path in paths {
        lines.extend(read_log_lines(&path)?);
    }

    if lines.len() > line_count {
        lines = lines.split_off(lines.len() - line_count);
    }

    Ok(SourceTail {
        source,
        available,
        lines,
    })
}

fn log_paths_for_initial_tail(active_path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, ExecuteError> {
    let mut paths = rotated_log_paths(active_path)?;

    if state::fs::path_exists(active_path) {
        paths.push(active_path.to_path_buf());
    }

    Ok(paths)
}

fn rotated_log_paths(active_path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, ExecuteError> {
    let Some(parent) = active_path.parent() else {
        return Ok(Vec::new());
    };
    let Some(file_name) = active_path.file_name() else {
        return Ok(Vec::new());
    };
    let rotated_prefix = format!("{file_name}.");
    let plain_rotated_prefix = file_name
        .strip_suffix(".log")
        .map(|stem| format!("{stem}-"));
    let mut paths = state::fs::read_dir_paths(parent)?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .map(|candidate| {
                    candidate.starts_with(&rotated_prefix)
                        || plain_rotated_prefix.as_deref().is_some_and(|prefix| {
                            candidate.starts_with(prefix) && candidate.ends_with(".log")
                        })
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    paths.sort();

    Ok(paths)
}

fn read_log_lines(path: &Utf8Path) -> Result<Vec<String>, ExecuteError> {
    match state::fs::read_to_string(path) {
        Ok(content) => Ok(content.lines().map(ToOwned::to_owned).collect()),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(Vec::new())
        }
        Err(error) => Err(error.into()),
    }
}

/// The source prefix PV adds before each stored log line when streams are
/// combined; empty for a single stream. Only this prefix may be colored: the
/// stored body is always written unchanged after it.
fn log_prefix(label: &str, prefixed: bool, color_enabled: bool) -> String {
    if !prefixed {
        return String::new();
    }

    format!(
        "{}{}",
        source_tone(label).paint(label, color_enabled),
        Tone::Dim.paint(" | ", color_enabled)
    )
}

fn source_tone(label: &str) -> Tone {
    if label.starts_with("daemon") {
        Tone::Value
    } else if label.starts_with("launchd") {
        Tone::Accent
    } else if label.starts_with("gateway") {
        Tone::Strong
    } else {
        Tone::Success
    }
}

fn follow_sources(
    sources: &[LogSource],
    color_enabled: bool,
    stdout: &mut dyn Write,
) -> Result<(), ExecuteError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(follow_sources_async(sources, color_enabled, stdout, None))
}

async fn follow_sources_async(
    sources: &[LogSource],
    color_enabled: bool,
    stdout: &mut dyn Write,
    max_lines: Option<usize>,
) -> Result<(), ExecuteError> {
    let mut muxed_lines = linemux::MuxedLines::new()?;
    let prefixed = sources.len() > 1;
    let mut prefixes = BTreeMap::new();

    for source in sources {
        let path = muxed_lines
            .add_file(source.active_path.as_std_path())
            .await?;
        prefixes.insert(path, log_prefix(&source.label, prefixed, color_enabled));
    }

    let unknown_source = log_prefix("log", prefixed, color_enabled);
    let mut emitted_lines = 0usize;
    while let Some(line) = muxed_lines.next_line().await? {
        let prefix = prefixes.get(line.source()).unwrap_or(&unknown_source);
        writeln!(stdout, "{prefix}{}", line.line())?;
        stdout.flush()?;
        emitted_lines += 1;
        if let Some(max_lines) = max_lines
            && emitted_lines >= max_lines
        {
            break;
        }
    }

    Ok(())
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use camino_tempfile::tempdir;
    use insta::assert_snapshot;

    use super::*;

    const BODIES: [&str; 4] = [
        "error: upstream worker:8.3 refused for legacy-shop.test",
        r#"{"level":"fatal","msg":"allowed memory size exhausted"}"#,
        "\u{1b}[31mnot PV color\u{1b}[0m [Warning] debug trace",
        "plain line",
    ];

    fn render(prefixed: bool, color: bool) -> String {
        let prefix = log_prefix("gateway:error", prefixed, color);
        BODIES
            .iter()
            .map(|body| format!("{prefix}{body}"))
            .collect::<Vec<_>>()
            .join("\n")
            .replace('\u{1b}', "␛")
    }

    #[test]
    fn log_bodies_are_emitted_unchanged_even_with_color() {
        assert_eq!(log_prefix("daemon", false, true), "");
        assert_snapshot!(render(false, true), @r#"
        error: upstream worker:8.3 refused for legacy-shop.test
        {"level":"fatal","msg":"allowed memory size exhausted"}
        ␛[31mnot PV color␛[0m [Warning] debug trace
        plain line
        "#);
        assert_snapshot!(render(true, true), @r#"
        ␛[1mgateway:error␛[0m␛[2m | ␛[0merror: upstream worker:8.3 refused for legacy-shop.test
        ␛[1mgateway:error␛[0m␛[2m | ␛[0m{"level":"fatal","msg":"allowed memory size exhausted"}
        ␛[1mgateway:error␛[0m␛[2m | ␛[0m␛[31mnot PV color␛[0m [Warning] debug trace
        ␛[1mgateway:error␛[0m␛[2m | ␛[0mplain line
        "#);
    }

    #[test]
    fn plain_log_prefixes_carry_no_escapes() {
        assert_snapshot!(render(true, false), @r#"
        gateway:error | error: upstream worker:8.3 refused for legacy-shop.test
        gateway:error | {"level":"fatal","msg":"allowed memory size exhausted"}
        gateway:error | ␛[31mnot PV color␛[0m [Warning] debug trace
        gateway:error | plain line
        "#);
    }

    #[test]
    fn follow_sources_multiplexes_active_files() -> anyhow::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        runtime.block_on(async {
            let tempdir = tempdir()?;
            let daemon_log = tempdir.path().join("daemon.log");
            let gateway_log = tempdir.path().join("gateway.log");
            write_test_file(&daemon_log, "")?;
            write_test_file(&gateway_log, "")?;
            let sources = vec![
                LogSource {
                    label: "daemon".to_string(),
                    active_path: daemon_log.clone(),
                },
                LogSource {
                    label: "gateway".to_string(),
                    active_path: gateway_log.clone(),
                },
            ];
            let writer = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(250)).await;
                append_test_file(&daemon_log, "daemon follow\n")?;
                append_test_file(&gateway_log, "gateway follow\n")?;

                anyhow::Ok(())
            });
            let mut stdout = Vec::new();

            follow_sources_async(&sources, false, &mut stdout, Some(2)).await?;
            writer.await??;

            let output = String::from_utf8(stdout)?;
            let mut lines = output.lines().collect::<Vec<_>>();
            lines.sort();

            assert_eq!(
                lines,
                vec!["daemon | daemon follow", "gateway | gateway follow"]
            );

            anyhow::Ok(())
        })
    }

    #[expect(clippy::disallowed_methods, reason = "logs tests create fixture files")]
    fn write_test_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
        std::fs::write(path, contents)?;

        Ok(())
    }

    #[expect(
        clippy::disallowed_types,
        reason = "logs tests append fixture log lines"
    )]
    fn append_test_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
        let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
        file.write_all(contents.as_bytes())?;

        Ok(())
    }
}
