use std::slice;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use platform::hooks::{self, HookCommandContext, HookEnvironment, HookError, HookOutput};
use state::{
    Database, JobDiagnosticSubject, JobStatus, ProjectRecord, PvPaths, RuntimeLifecycleLock,
    StateError,
};
use tokio::sync::watch;

use crate::error::{CliError, ExecuteError};
use crate::output::Streams;

/// One terminal invocation captures its environment once across prepare and setup.
pub(super) struct LinkHookRun<'a> {
    paths: &'a PvPaths,
    project_path: &'a Utf8Path,
    source_path: Option<Utf8PathBuf>,
    captured: Option<HookEnvironment>,
    database_existed: bool,
    runtime: tokio::runtime::Runtime,
}

impl<'a> LinkHookRun<'a> {
    pub(super) fn new(
        paths: &'a PvPaths,
        project_path: &'a Utf8Path,
        source_path: Option<Utf8PathBuf>,
        database_existed: bool,
    ) -> Result<Self, ExecuteError> {
        Ok(Self {
            paths,
            project_path,
            source_path,
            captured: None,
            database_existed,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        })
    }

    pub(super) fn run(
        &mut self,
        phase: &'static str,
        commands: &[String],
        streams: &mut Streams<'_>,
    ) -> Result<(), ExecuteError> {
        let result = self.runtime.block_on(async {
            let (sender, mut shutdown) = watch::channel(false);
            let operation = async {
                let captured = match self.captured.take() {
                    Some(captured) => captured,
                    None => {
                        hooks::capture_environment(
                            &hooks::login_shell()?,
                            self.paths.home(),
                            self.project_path,
                            &mut shutdown,
                        )
                        .await?
                    }
                };
                let context = HookCommandContext {
                    paths: self.paths,
                    project_path: self.project_path,
                    source_path: self.source_path.as_deref(),
                    environment: &captured,
                    output: HookOutput::Terminal,
                };
                let result =
                    hooks::run_commands(&context, phase, commands, &mut shutdown, |announcement| {
                        streams.out.line(announcement)?;
                        streams.out.writer().flush()
                    })
                    .await;
                self.captured = Some(captured);
                result
            };
            tokio::pin!(operation);
            tokio::select! {
                biased;
                interrupt = tokio::signal::ctrl_c() => {
                    sender.send_replace(true);
                    let stopped = operation.await;
                    interrupt.map_err(HookError::from)?;
                    stopped.and(Err(HookError::Interrupted))
                }
                result = &mut operation => result,
            }
        });
        result.map_err(|source| ExecuteError::ProjectHook { phase, source })
    }

    pub(super) fn wait_for_reconciliation(
        &self,
        job_id: &str,
        streams: &mut Streams<'_>,
    ) -> Result<(), ExecuteError> {
        streams
            .out
            .line(&format!("Waiting for reconciliation {job_id} before setup"))?;
        streams.out.writer().flush()?;
        self.runtime.block_on(async {
            tokio::select! {
                biased;
                interrupt = tokio::signal::ctrl_c() => {
                    interrupt?;
                    Err(ExecuteError::ProjectHook { phase: "setup", source: HookError::Interrupted })
                }
                result = wait_for_reconciliation_job(self.paths, job_id) => result,
            }
        })
    }

    pub(super) fn request_reconciliation(
        &self,
        project: &ProjectRecord,
        streams: &mut Streams<'_>,
        hooks_enabled: bool,
    ) -> Result<Option<daemon::SubmittedJob>, ExecuteError> {
        if !hooks_enabled {
            return self
                .runtime
                .block_on(super::request_project_reconciliation_async(
                    self.paths, project, streams,
                ));
        }
        self.runtime.block_on(async {
            tokio::select! {
                biased;
                interrupt = tokio::signal::ctrl_c() => {
                    interrupt?;
                    Err(ExecuteError::ProjectHook { phase: "reconciliation", source: HookError::Interrupted })
                }
                result = super::request_project_reconciliation_async(self.paths, project, streams) => result,
            }
        })
    }
}

/// An outer error forbids further writes and IPC; the inner hook outcome may be recorded after
/// admission has been reacquired.
pub(super) fn run_with_admission(
    run: &mut LinkHookRun<'_>,
    phase: &'static str,
    commands: &[String],
    streams: &mut Streams<'_>,
    admission: &mut Option<RuntimeLifecycleLock>,
) -> Result<Result<(), ExecuteError>, ExecuteError> {
    let had_admission = admission.is_some();
    drop(admission.take());
    let result = run.run(phase, commands, streams);
    // Reacquire even after failure. A disable/uninstall owner ends the command before any further state
    // write or daemon request, including recording the hook outcome.
    let resume = if had_admission {
        RuntimeLifecycleLock::acquire_shared(run.paths)
            .map(|guard| *admission = Some(guard))
            .map_err(super::coordination_lock_error)
    } else {
        Ok(())
    }
    .and_then(|()| {
        if run.database_existed && !state::fs::path_entry_exists(run.paths.db())? {
            return Err(CliError::LinkDatabaseRemoved.into());
        }
        Ok(())
    });
    if let Err(resume) = resume {
        return Err(match result {
            Ok(()) => resume,
            Err(original) => ExecuteError::HookResume {
                original: Box::new(original),
                resume: Box::new(resume),
            },
        });
    }
    Ok(result)
}

async fn wait_for_reconciliation_job(paths: &PvPaths, job_id: &str) -> Result<(), ExecuteError> {
    let database = Database::open(paths)?;
    loop {
        let job = database
            .job(job_id)?
            .ok_or_else(|| CliError::LinkReconciliationUnknown {
                job_id: job_id.to_string(),
            })?;
        match job.status {
            JobStatus::Running => tokio::time::sleep(Duration::from_millis(100)).await,
            JobStatus::Succeeded => return Ok(()),
            JobStatus::Failed => {
                return Err(CliError::LinkReconciliationFailed {
                    job_id: job_id.to_string(),
                    message: job.error.unwrap_or_else(|| "unknown failure".to_string()),
                }
                .into());
            }
        }
    }
}

fn record_run(
    paths: &PvPaths,
    project: &ProjectRecord,
    result: &Result<(), ExecuteError>,
    verified: bool,
) -> Result<(), StateError> {
    let scope = format!("project:{}", project.id);
    let subject = JobDiagnosticSubject::Other {
        kind: "hooks".to_string(),
        scope: scope.clone(),
    };
    let mut database = Database::open(paths)?;
    let error = result.as_ref().err().map(ToString::to_string);
    let outcome = match error.as_deref() {
        Some(error) => Err(error),
        None if verified => Ok("Project hooks completed"),
        None => Ok("Prepare hooks completed; setup skipped"),
    };
    let subjects = if verified || result.is_err() {
        slice::from_ref(&subject)
    } else {
        &[]
    };
    database.record_finished_job("hooks", &scope, outcome, subjects)?;
    Ok(())
}

pub(super) fn finish_run(
    paths: &PvPaths,
    project: &ProjectRecord,
    result: Result<(), ExecuteError>,
    verified: bool,
) -> Result<(), ExecuteError> {
    match (record_run(paths, project, &result, verified), result) {
        (Ok(()), result) => result,
        (Err(recording), Ok(())) => Err(recording.into()),
        (Err(recording), Err(original)) => Err(ExecuteError::HookOutcomeRecording {
            original: Box::new(original),
            recording,
        }),
    }
}
