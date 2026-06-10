use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use serde::Serialize;
use state::{Database, JobRecord, JobStatus, PvPaths, StateError};

use crate::args::JobsArgs;
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

pub(crate) fn run(
    args: JobsArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let jobs = database.recent_jobs()?;

    if args.json {
        serde_json::to_writer(&mut *stdout, &JobsJson::from_records(&jobs))?;
        writeln!(stdout)?;

        return Ok(ExitCode::SUCCESS);
    }

    let mut output = Output::new(stdout, OutputMode::plain());
    write_jobs(&jobs, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

fn write_jobs(jobs: &[JobRecord], output: &mut Output<'_, impl Write>) -> Result<(), ExecuteError> {
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
