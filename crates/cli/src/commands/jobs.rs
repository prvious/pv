use std::process::ExitCode;

use camino::Utf8PathBuf;
use serde::Serialize;
use state::{Database, JobRecord, JobStatus, PvPaths, StateError};

use crate::args::JobsArgs;
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Output, Streams};

pub(crate) fn run(
    args: JobsArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let jobs = match Database::open_read_only(&paths)? {
        Some(database) => database.recent_jobs()?,
        None => Vec::new(),
    };

    if args.json {
        streams.out.json(&JobsJson::from_records(&jobs))?;

        return Ok(ExitCode::SUCCESS);
    }

    write_jobs(&jobs, &mut streams.out)?;

    Ok(ExitCode::SUCCESS)
}

fn write_jobs(jobs: &[JobRecord], output: &mut Output<'_>) -> Result<(), ExecuteError> {
    if jobs.is_empty() {
        output.line("No recent daemon jobs")?;
        return Ok(());
    }

    output.line("ID  Kind  Scope  Status  Started  Finished  Summary")?;
    for job in jobs {
        output.line(&format!(
            "{}  {}  {}  {}  {}  {}  {}",
            job.id,
            job.kind,
            job.scope,
            job_status_label(job.status),
            job.started_at,
            job.finished_at.as_deref().unwrap_or("-"),
            job_summary(job),
        ))?;
    }

    Ok(())
}

fn job_summary(job: &JobRecord) -> &str {
    job.error
        .as_deref()
        .or(job.summary.as_deref())
        .unwrap_or("-")
}

fn job_status_label(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Running => "running",
        JobStatus::Succeeded => "succeeded",
        JobStatus::Failed => "failed",
    }
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

#[derive(Serialize)]
struct JobsJson<'job> {
    jobs: Vec<JobJson<'job>>,
}

impl<'job> JobsJson<'job> {
    fn from_records(jobs: &'job [JobRecord]) -> Self {
        Self {
            jobs: jobs.iter().map(JobJson::from_record).collect(),
        }
    }
}

#[derive(Serialize)]
struct JobJson<'job> {
    id: &'job str,
    kind: &'job str,
    scope: &'job str,
    status: &'static str,
    started_at: &'job str,
    finished_at: Option<&'job str>,
    summary: Option<&'job str>,
    error: Option<&'job str>,
}

impl<'job> JobJson<'job> {
    fn from_record(job: &'job JobRecord) -> Self {
        Self {
            id: &job.id,
            kind: &job.kind,
            scope: &job.scope,
            status: job_status_label(job.status),
            started_at: &job.started_at,
            finished_at: job.finished_at.as_deref(),
            summary: job.summary.as_deref(),
            error: job.error.as_deref(),
        }
    }
}
