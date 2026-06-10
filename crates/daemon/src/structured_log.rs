use std::io::Write as _;

use serde_json::{Map, Value};
use state::PvPaths;

use crate::DaemonError;

pub(crate) fn daemon_started(paths: &PvPaths) -> Result<(), DaemonError> {
    append(
        paths,
        "info",
        "daemon",
        "daemon_started",
        "daemon started",
        &[],
    )
}

pub(crate) fn daemon_stopped(paths: &PvPaths) -> Result<(), DaemonError> {
    append(
        paths,
        "info",
        "daemon",
        "daemon_stopped",
        "daemon stopped",
        &[],
    )
}

pub(crate) fn job_started(
    paths: &PvPaths,
    job_id: &str,
    kind: &str,
    scope: &str,
) -> Result<(), DaemonError> {
    append(
        paths,
        "info",
        "reconciliation",
        "job_started",
        "job started",
        &[("job_id", job_id), ("kind", kind), ("scope", scope)],
    )
}

pub(crate) fn job_completed(
    paths: &PvPaths,
    job_id: &str,
    kind: &str,
    scope: &str,
    summary: &str,
) -> Result<(), DaemonError> {
    append(
        paths,
        "info",
        "reconciliation",
        "job_completed",
        summary,
        &[
            ("job_id", job_id),
            ("kind", kind),
            ("scope", scope),
            ("summary", summary),
        ],
    )
}

pub(crate) fn job_failed(
    paths: &PvPaths,
    job_id: &str,
    kind: &str,
    scope: &str,
    error: &str,
) -> Result<(), DaemonError> {
    append(
        paths,
        "error",
        "reconciliation",
        "job_failed",
        error,
        &[
            ("job_id", job_id),
            ("kind", kind),
            ("scope", scope),
            ("error", error),
        ],
    )
}

fn append(
    paths: &PvPaths,
    level: &str,
    target: &str,
    event: &str,
    message: &str,
    fields: &[(&str, &str)],
) -> Result<(), DaemonError> {
    let mut record = Map::new();
    record.insert("timestamp".to_string(), Value::String(timestamp()?));
    record.insert("level".to_string(), Value::String(level.to_string()));
    record.insert("target".to_string(), Value::String(target.to_string()));
    record.insert("event".to_string(), Value::String(event.to_string()));
    record.insert("message".to_string(), Value::String(message.to_string()));

    for (key, value) in fields {
        record.insert((*key).to_string(), Value::String((*value).to_string()));
    }

    let mut file = state::fs::open_append_file(&paths.daemon_log())?;
    writeln!(file, "{}", Value::Object(record))?;

    Ok(())
}

fn timestamp() -> Result<String, DaemonError> {
    let format =
        time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

    Ok(time::OffsetDateTime::now_utc().format(format)?)
}
