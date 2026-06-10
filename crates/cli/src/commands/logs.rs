use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::io::Write;
use std::process::ExitCode;

use anstyle::{AnsiColor, Style};
use camino::{Utf8Path, Utf8PathBuf};
use state::{Database, PvPaths, StateError};

use crate::args::LogsArgs;
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

const MAX_LINE_COUNT: usize = 5000;

pub(crate) fn run(
    args: LogsArgs,
    no_color: bool,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let line_count = line_count(args.lines)?;
    let paths = pv_paths(environment)?;
    let selection = select_sources(&args, &paths)?;
    let color_enabled =
        !no_color && environment.var_os("NO_COLOR").is_none() && environment.stdout_is_terminal();
    let mut output = Output::new(stdout, OutputMode::from_no_color(no_color));

    write_initial_tail(
        &selection.sources,
        line_count,
        &selection.empty_message,
        color_enabled,
        &mut output,
    )?;

    if args.follow {
        follow_sources(&selection.sources, color_enabled, stdout)?;
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
        return Ok(LogSelection {
            sources: vec![LogSource {
                label: format!("worker:{worker}"),
                active_path: paths.worker_log(worker),
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

    if state::fs::path_exists(&access) || state::fs::path_exists(&error) {
        return vec![
            LogSource {
                label: "gateway:access".to_string(),
                active_path: access,
            },
            LogSource {
                label: "gateway:error".to_string(),
                active_path: error,
            },
        ];
    }

    vec![LogSource {
        label: "gateway".to_string(),
        active_path: paths.gateway_log(),
    }]
}

fn installed_worker_sources(paths: &PvPaths) -> Result<Vec<LogSource>, ExecuteError> {
    let database = Database::open(paths)?;
    let mut tracks = BTreeSet::new();

    for state in database.runtime_observed_states()? {
        if let state::RuntimeSubject::PhpWorker { php_track } = state.subject {
            tracks.insert(php_track);
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
    let database = Database::open(paths)?;

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
        return Ok(track.to_string());
    }

    let database = Database::open(paths)?;
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

fn write_initial_tail(
    sources: &[LogSource],
    line_count: usize,
    empty_message: &str,
    color_enabled: bool,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    let tails = sources
        .iter()
        .map(|source| source_tail(source, line_count))
        .collect::<Result<Vec<_>, _>>()?;
    let available_count = tails.iter().filter(|tail| tail.available).count();

    if available_count == 0 {
        output.line(empty_message)?;
        return Ok(());
    }

    let prefix = available_count > 1;
    for tail in tails {
        for line in tail.lines {
            output.line(&format_log_line(
                &tail.source.label,
                &line,
                prefix,
                color_enabled,
            ))?;
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
    let mut paths = state::fs::read_dir_paths(parent)?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .map(|candidate| candidate.starts_with(&rotated_prefix))
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

fn format_log_line(label: &str, line: &str, prefix: bool, color_enabled: bool) -> String {
    let line = color_severity(line, color_enabled);

    if !prefix {
        return line;
    }

    format!("{} | {line}", color_label(label, color_enabled))
}

fn color_label(label: &str, color_enabled: bool) -> String {
    if !color_enabled {
        return label.to_string();
    }

    let style = if label.starts_with("daemon") {
        Style::new().fg_color(Some(AnsiColor::Cyan.into()))
    } else if label.starts_with("launchd") {
        Style::new().fg_color(Some(AnsiColor::Magenta.into()))
    } else if label.starts_with("gateway") {
        Style::new().fg_color(Some(AnsiColor::Blue.into()))
    } else {
        Style::new().fg_color(Some(AnsiColor::Green.into()))
    };

    format!("{style}{label}{style:#}")
}

fn color_severity(line: &str, color_enabled: bool) -> String {
    if !color_enabled {
        return line.to_string();
    }

    let lowercase = line.to_ascii_lowercase();
    let style = if lowercase.contains("error") || lowercase.contains("fatal") {
        Some(Style::new().fg_color(Some(AnsiColor::Red.into())))
    } else if lowercase.contains("warn") || lowercase.contains("warning") {
        Some(Style::new().fg_color(Some(AnsiColor::Yellow.into())))
    } else if lowercase.contains("debug") || lowercase.contains("trace") {
        Some(Style::new().dimmed())
    } else {
        None
    };

    match style {
        Some(style) => format!("{style}{line}{style:#}"),
        None => line.to_string(),
    }
}

fn follow_sources(
    sources: &[LogSource],
    color_enabled: bool,
    stdout: &mut impl Write,
) -> Result<(), ExecuteError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(follow_sources_async(sources, color_enabled, stdout, None))
}

async fn follow_sources_async(
    sources: &[LogSource],
    color_enabled: bool,
    stdout: &mut impl Write,
    max_lines: Option<usize>,
) -> Result<(), ExecuteError> {
    let mut muxed_lines = linemux::MuxedLines::new()?;
    let mut labels = BTreeMap::new();

    for source in sources {
        let path = muxed_lines
            .add_file(source.active_path.as_std_path())
            .await?;
        labels.insert(path, source.label.clone());
    }

    let prefix = sources.len() > 1;
    let mut emitted_lines = 0usize;
    while let Some(line) = muxed_lines.next_line().await? {
        let label = labels
            .get(line.source())
            .map(String::as_str)
            .unwrap_or("log");
        writeln!(
            stdout,
            "{}",
            format_log_line(label, line.line(), prefix, color_enabled)
        )?;
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
    use std::io::Write as _;
    use std::time::Duration;

    use camino_tempfile::tempdir;

    use super::*;

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
