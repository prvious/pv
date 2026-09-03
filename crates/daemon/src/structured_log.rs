use std::io::{self, Write as _};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use state::PvPaths;

use crate::DaemonError;

#[derive(Clone, Debug)]
pub(crate) struct ReconciliationPhaseLog {
    paths: PvPaths,
    job_id: String,
    scope: String,
}

#[derive(Clone, Copy)]
pub(crate) enum PhaseOutcome {
    Succeeded,
    Failed,
    Skipped,
    Fallback,
}

#[derive(Clone, Copy)]
pub(crate) enum ReconciliationPhase {
    Queue,
    DemandDiscovery,
    Manifest,
    Download,
    Install,
    ProjectApply,
    Resources,
    Workers,
    Gateway,
    Finalization,
}

pub(crate) struct PhaseTimer {
    log: ReconciliationPhaseLog,
    phase: ReconciliationPhase,
    subject: String,
    started_at: Instant,
    finished: bool,
}

impl ReconciliationPhaseLog {
    pub(crate) fn new(paths: &PvPaths, job_id: &str, scope: &str) -> Self {
        Self {
            paths: paths.clone(),
            job_id: job_id.to_owned(),
            scope: scope.to_owned(),
        }
    }

    pub(crate) fn start(
        &self,
        phase: ReconciliationPhase,
        subject: impl Into<String>,
    ) -> PhaseTimer {
        PhaseTimer {
            log: self.clone(),
            phase,
            subject: subject.into(),
            started_at: Instant::now(),
            finished: false,
        }
    }

    pub(crate) fn completed(
        &self,
        phase: ReconciliationPhase,
        subject: &str,
        outcome: PhaseOutcome,
        elapsed: Duration,
        counts: &[(&str, u64)],
    ) {
        self.completed_with_fields(phase, subject, outcome, elapsed, counts, &[]);
    }

    pub(crate) fn completed_with_fields(
        &self,
        phase: ReconciliationPhase,
        subject: &str,
        outcome: PhaseOutcome,
        elapsed: Duration,
        counts: &[(&str, u64)],
        fields: &[(&str, &str)],
    ) {
        if let Err(error) = append_phase(self, phase, subject, outcome, elapsed, counts, fields) {
            let mut standard_error = io::stderr().lock();
            let _fallback_result = writeln!(
                standard_error,
                "PV reconciliation phase log failed: job_id={} phase={} subject={subject}: {error}",
                self.job_id,
                phase.as_str(),
            );
        }
    }
}

impl PhaseOutcome {
    pub(crate) fn from_succeeded(succeeded: bool) -> Self {
        if succeeded {
            Self::Succeeded
        } else {
            Self::Failed
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Fallback => "fallback",
        }
    }
}

impl ReconciliationPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::DemandDiscovery => "demand_discovery",
            Self::Manifest => "manifest",
            Self::Download => "download",
            Self::Install => "install",
            Self::ProjectApply => "project_apply",
            Self::Resources => "resources",
            Self::Workers => "workers",
            Self::Gateway => "gateway",
            Self::Finalization => "finalization",
        }
    }
}

impl PhaseTimer {
    pub(crate) fn finish(mut self, outcome: PhaseOutcome, counts: &[(&str, u64)]) {
        self.finished = true;
        self.log.completed(
            self.phase,
            &self.subject,
            outcome,
            self.started_at.elapsed(),
            counts,
        );
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        if self.finished {
            return;
        }

        self.log.completed(
            self.phase,
            &self.subject,
            PhaseOutcome::Failed,
            self.started_at.elapsed(),
            &[],
        );
    }
}

pub(crate) fn daemon_started(paths: &PvPaths) {
    append_best_effort(
        paths,
        "info",
        "daemon",
        "daemon_started",
        "daemon started",
        &[],
    );
}

pub(crate) fn daemon_stopped(paths: &PvPaths) {
    append_best_effort(
        paths,
        "info",
        "daemon",
        "daemon_stopped",
        "daemon stopped",
        &[],
    );
}

pub(crate) fn job_started(paths: &PvPaths, job_id: &str, kind: &str, scope: &str) {
    append_best_effort(
        paths,
        "info",
        "reconciliation",
        "job_started",
        "job started",
        &[("job_id", job_id), ("kind", kind), ("scope", scope)],
    );
}

pub(crate) fn job_completed(paths: &PvPaths, job_id: &str, kind: &str, scope: &str, summary: &str) {
    append_best_effort(
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
    );
}

pub(crate) fn job_failed(paths: &PvPaths, job_id: &str, kind: &str, scope: &str, error: &str) {
    append_best_effort(
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
    );
}

pub(crate) fn runtime_readiness_diagnostics(
    paths: &PvPaths,
    runtime: &str,
    readiness: &str,
    process_exited: &str,
    loopback_listener_ports: &str,
) {
    append_best_effort(
        paths,
        "error",
        "runtime",
        "runtime_readiness_diagnostics",
        "runtime readiness failed diagnostics",
        &[
            ("runtime", runtime),
            ("readiness", readiness),
            ("process_exited", process_exited),
            ("loopback_listener_ports", loopback_listener_ports),
        ],
    );
}

pub(crate) fn runtime_config_cleanup_failed(paths: &PvPaths, runtime: &str, error: &str) {
    append_best_effort(
        paths,
        "warn",
        "runtime",
        "runtime_config_cleanup_failed",
        "runtime config committed but backup cleanup failed",
        &[("runtime", runtime), ("error", error)],
    );
}

pub(crate) fn project_tls_maintenance_failed(paths: &PvPaths, project_id: &str, error: &str) {
    append_best_effort(
        paths,
        "error",
        "reconciliation",
        "project_tls_maintenance_failed",
        "Project TLS maintenance failed",
        &[("project_id", project_id), ("error", error)],
    );
}

fn append_best_effort(
    paths: &PvPaths,
    level: &str,
    target: &str,
    event: &str,
    message: &str,
    fields: &[(&str, &str)],
) {
    let _append_result = append(paths, level, target, event, message, fields);
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

fn append_phase(
    log: &ReconciliationPhaseLog,
    phase: ReconciliationPhase,
    subject: &str,
    outcome: PhaseOutcome,
    elapsed: Duration,
    counts: &[(&str, u64)],
    fields: &[(&str, &str)],
) -> Result<(), DaemonError> {
    let mut record = Map::new();
    for (key, value) in fields {
        record.insert((*key).to_string(), Value::String((*value).to_string()));
    }
    for (key, value) in counts {
        record.insert((*key).to_string(), Value::from(*value));
    }
    record.insert("timestamp".to_string(), Value::String(timestamp()?));
    record.insert("level".to_string(), Value::String("info".to_string()));
    record.insert(
        "target".to_string(),
        Value::String("reconciliation".to_string()),
    );
    record.insert(
        "event".to_string(),
        Value::String("reconciliation_phase_completed".to_string()),
    );
    record.insert(
        "message".to_string(),
        Value::String("reconciliation phase completed".to_string()),
    );
    record.insert("job_id".to_string(), Value::String(log.job_id.clone()));
    record.insert("kind".to_string(), Value::String("reconcile".to_string()));
    record.insert("scope".to_string(), Value::String(log.scope.clone()));
    record.insert(
        "phase".to_string(),
        Value::String(phase.as_str().to_string()),
    );
    record.insert(
        "outcome".to_string(),
        Value::String(outcome.as_str().to_string()),
    );
    record.insert(
        "elapsed_ms".to_string(),
        Value::from(elapsed_milliseconds(elapsed)),
    );
    record.insert("subject".to_string(), Value::String(subject.to_string()));
    let mut file = state::fs::open_append_file(&log.paths.daemon_log())?;
    writeln!(file, "{}", Value::Object(record))?;

    Ok(())
}

fn elapsed_milliseconds(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

fn timestamp() -> Result<String, DaemonError> {
    let format =
        time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

    Ok(time::OffsetDateTime::now_utc().format(format)?)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use camino_tempfile::tempdir;
    use serde_json::Value;
    use state::PvPaths;

    use super::{PhaseOutcome, ReconciliationPhase, ReconciliationPhaseLog};

    #[test]
    fn reconciliation_phase_log_and_timer_record_each_completion_once() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let log = ReconciliationPhaseLog::new(&paths, "job_1", "system");

        log.completed(
            ReconciliationPhase::Manifest,
            "artifact_manifest",
            PhaseOutcome::Succeeded,
            Duration::from_millis(12),
            &[("artifact_count", 3)],
        );
        log.start(ReconciliationPhase::Download, "php:8.5")
            .finish(PhaseOutcome::Succeeded, &[("artifact_count", 1)]);
        drop(log.start(ReconciliationPhase::Install, "php:8.5"));

        let events = state::fs::read_to_string(&paths.daemon_log())?
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["event"], "reconciliation_phase_completed");
        assert_eq!(events[0]["job_id"], "job_1");
        assert_eq!(events[0]["kind"], "reconcile");
        assert_eq!(events[0]["scope"], "system");
        assert_eq!(events[0]["phase"], "manifest");
        assert_eq!(events[0]["outcome"], "succeeded");
        assert_eq!(events[0]["elapsed_ms"].as_u64(), Some(12));
        assert_eq!(events[0]["subject"], "artifact_manifest");
        assert_eq!(events[0]["artifact_count"], 3);
        assert_eq!(events[1]["phase"], "download");
        assert_eq!(events[1]["outcome"], "succeeded");
        assert!(events[1]["elapsed_ms"].as_u64().is_some());
        assert_eq!(events[2]["phase"], "install");
        assert_eq!(events[2]["outcome"], "failed");
        assert!(events[2]["elapsed_ms"].as_u64().is_some());

        Ok(())
    }
}
