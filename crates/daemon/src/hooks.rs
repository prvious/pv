//! Daemon hook runs have their own FIFO and never enter the reconciliation queue.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use camino::{Utf8Path, Utf8PathBuf};
use config::ProjectHooks;
use platform::hooks::{self, HookCommandContext, HookError, HookOutput};
use state::{Database, JobDiagnosticSubject, PvPaths};
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;

use crate::DaemonError;
use crate::reconciliation::ReconciliationDebouncer;
use crate::{structured_log, worktrees};

const CONCURRENCY: usize = 4;

#[derive(Clone)]
pub(crate) struct DaemonHooks {
    paths: PvPaths,
    inner: Arc<Mutex<HookState>>,
    changed: Arc<Notify>,
}

#[derive(Default)]
struct HookState {
    stopping: bool,
    queued: VecDeque<HookRun>,
    active: BTreeMap<String, ActiveRun>,
    prepares: BTreeMap<String, PrepareState>,
}

struct PrepareState {
    path: Utf8PathBuf,
    source_project_id: String,
    worktree_id: String,
    outcome: PrepareOutcome,
}

#[derive(Clone, Copy, PartialEq)]
enum PrepareOutcome {
    Pending,
    Prepared,
    Failed,
}

struct ActiveRun {
    run: HookRun,
    cancel: watch::Sender<bool>,
}

#[derive(Clone)]
struct HookRun {
    job_id: String,
    scope: String,
    phase: &'static str,
    commands: Vec<String>,
    /// Every command in both lists; the run needs all of them approved.
    approval_commands: Vec<String>,
    project_path: Utf8PathBuf,
    source_path: Utf8PathBuf,
    log: Utf8PathBuf,
    project_id: Option<String>,
    worktree_id: String,
    source_project_id: String,
}

impl DaemonHooks {
    pub(crate) fn new(paths: PvPaths) -> Self {
        Self {
            paths,
            inner: Arc::default(),
            changed: Arc::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, HookState> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn begin_shutdown(&self) {
        let mut state = self.state();
        state.stopping = true;
        for active in state.active.values() {
            active.cancel.send_replace(true);
        }
    }

    pub(crate) fn prepare_is_blocked(&self, source: &str, worktree: &str) -> bool {
        self.state()
            .prepares
            .get(&worktree_scope(source, worktree))
            .is_some_and(|prepare| prepare.outcome != PrepareOutcome::Prepared)
    }

    pub(crate) fn is_prepared(&self, source: &str, worktree: &str) -> bool {
        self.state()
            .prepares
            .get(&worktree_scope(source, worktree))
            .is_some_and(|prepare| prepare.outcome == PrepareOutcome::Prepared)
    }

    pub(crate) fn forget_prepare(&self, source: &str, worktree: &str) {
        self.state()
            .prepares
            .remove(&worktree_scope(source, worktree));
    }

    pub(crate) fn queue_prepare(
        &self,
        source_project_id: &str,
        worktree_id: &str,
        project_path: &Utf8Path,
        hooks: &ProjectHooks,
    ) -> Result<(), DaemonError> {
        let mut database = Database::open(&self.paths)?;
        let source = database.project_by_id(source_project_id)?.ok_or_else(|| {
            state::StateError::ProjectNotFound {
                target: source_project_id.to_owned(),
            }
        })?;
        let scope = worktree_scope(source_project_id, worktree_id);
        let mut state = self.state();
        if state.prepares.contains_key(&scope)
            || state.active.contains_key(&scope)
            || state.queued.iter().any(|run| run.scope == scope)
        {
            return Ok(());
        }
        let log = self.paths.worktree_hook_log(source_project_id, worktree_id);
        let job = database.start_job_with_summary(
            "hooks",
            &scope,
            Some(&format!("prepare hooks queued (log: {log})")),
        )?;
        state.prepares.insert(
            scope.clone(),
            PrepareState {
                path: project_path.to_owned(),
                source_project_id: source_project_id.to_owned(),
                worktree_id: worktree_id.to_owned(),
                outcome: PrepareOutcome::Pending,
            },
        );
        let run = HookRun {
            job_id: job.id,
            scope,
            phase: "prepare",
            commands: hooks.prepare.clone(),
            approval_commands: all_hook_commands(hooks),
            project_path: project_path.to_owned(),
            source_path: source.path,
            log,
            project_id: None,
            worktree_id: worktree_id.to_owned(),
            source_project_id: source_project_id.to_owned(),
        };
        if state.stopping {
            self.record_outcome(&run, &Err(HookError::Interrupted));
        } else {
            state.queued.push_back(run);
        }
        self.changed.notify_one();
        Ok(())
    }

    pub(crate) fn queue_setup(
        &self,
        project_id: &str,
        job_id: &str,
        hooks: &ProjectHooks,
    ) -> Result<(), DaemonError> {
        let mut database = Database::open(&self.paths)?;
        let discovered = database
            .discovered_projects()?
            .into_iter()
            .find(|project| project.project_id == project_id);
        let project = database.project_by_id(project_id)?;
        let source = if let Some(discovered) = &discovered {
            database.project_by_id(&discovered.source_project_id)?
        } else {
            None
        };
        let scope = format!("project:{project_id}");
        let log = self.paths.project_hook_log(project_id);
        let (Some(discovered), Some(project), Some(source)) = (discovered, project, source) else {
            database.fail_job_with_subject(
                job_id,
                &format!("setup interrupted before execution (log: {log})"),
                &JobDiagnosticSubject::Other {
                    kind: "hooks".to_string(),
                    scope: scope.clone(),
                },
            )?;
            return Ok(());
        };
        let mut state = self.state();
        let run = HookRun {
            job_id: job_id.to_string(),
            scope,
            phase: "setup",
            commands: hooks.setup.clone(),
            approval_commands: all_hook_commands(hooks),
            project_path: project.path,
            source_path: source.path,
            log,
            project_id: Some(project_id.to_owned()),
            worktree_id: discovered.worktree_id,
            source_project_id: discovered.source_project_id,
        };
        if state.stopping {
            self.record_outcome(&run, &Err(HookError::Interrupted));
        } else {
            state.queued.push_back(run);
        }
        self.changed.notify_one();
        Ok(())
    }

    /// Waits for obsolete runs to finish or report cleanup failure before allocation cleanup.
    pub(crate) async fn stop_obsolete(&self) -> Result<(), DaemonError> {
        loop {
            let notified = self.changed.notified();
            let pending = self.cancel_obsolete()?;
            if !pending {
                return Ok(());
            }
            notified.await;
        }
    }

    fn cancel_obsolete(&self) -> Result<bool, DaemonError> {
        let mut state = self.state();
        if state.queued.is_empty() && state.active.is_empty() && state.prepares.is_empty() {
            return Ok(false);
        }
        let mut database = Database::open(&self.paths)?;
        let mut obsolete = Vec::new();
        for (scope, prepare) in &state.prepares {
            let source = database.project_by_id(&prepare.source_project_id)?;
            let summary = if database.project_by_path(&prepare.path)?.is_some() {
                Some("prepare skipped: linked as an ordinary Project")
            } else if source
                .as_ref()
                .and_then(|source| {
                    self.registered_path(&source.path, &prepare.path, &prepare.worktree_id, scope)
                })
                .is_none()
            {
                Some("prepare skipped: worktree removed")
            } else {
                None
            };
            if let Some(summary) = summary {
                obsolete.push((scope.clone(), summary));
            }
        }
        for (scope, summary) in obsolete {
            self.discard_prepare(&mut state, &mut database, &scope, summary)?;
        }
        let mut obsolete = Vec::new();
        for run in state
            .queued
            .iter()
            .chain(state.active.values().map(|active| &active.run))
        {
            // Stop a removed worktree's setup at the first barrier, before discovery removes it.
            if let Some(project_id) = &run.project_id {
                let source = database.project_by_id(&run.source_project_id)?;
                if database.project_by_id(project_id)?.is_none()
                    || source
                        .as_ref()
                        .and_then(|source| {
                            self.registered_path(
                                &source.path,
                                &run.project_path,
                                &run.worktree_id,
                                &run.scope,
                            )
                        })
                        .is_none()
                {
                    obsolete.push(run.scope.clone());
                }
            }
        }
        for scope in obsolete {
            if let Some(active) = state.active.get(&scope) {
                active.cancel.send_replace(true);
            } else if let Some(index) = state.queued.iter().position(|run| run.scope == scope)
                && let Some(run) = state.queued.remove(index)
            {
                self.record_outcome(&run, &Err(HookError::Interrupted));
            }
        }
        Ok(state.active.values().any(|active| *active.cancel.borrow()))
    }

    fn discard_prepare(
        &self,
        state: &mut HookState,
        database: &mut Database,
        scope: &str,
        summary: &str,
    ) -> Result<(), DaemonError> {
        let Some(prepare) = state.prepares.get(scope) else {
            return Ok(());
        };
        let active = state.active.get(scope);
        if let Some(active) = active {
            active.cancel.send_replace(true);
        }
        let subject = JobDiagnosticSubject::Other {
            kind: "hooks".to_owned(),
            scope: scope.to_owned(),
        };
        if let Some(job) = database.active_hook_job(scope)? {
            database.complete_job_with_coverage(&job.id, summary, &[subject])?;
        } else if prepare.outcome == PrepareOutcome::Failed {
            database.record_finished_job("hooks", scope, Ok(summary), &[subject])?;
        }
        state.prepares.remove(scope);
        state.queued.retain(|run| run.scope != scope);
        if active.is_none()
            && let Some(log) = self.paths.hook_log_for_scope(scope)
        {
            delete_log(&log)?;
        }
        Ok(())
    }

    fn ready_to_run(&self, run: &mut HookRun) -> Result<bool, DaemonError> {
        let mut database = Database::open(&self.paths)?;
        let current = if let Some(project_id) = &run.project_id {
            database
                .project_by_id(project_id)?
                .map(|project| project.path)
        } else {
            database
                .project_by_id(&run.source_project_id)?
                .and_then(|source| {
                    self.registered_path(
                        &source.path,
                        &run.project_path,
                        &run.worktree_id,
                        &run.scope,
                    )
                })
        };
        let summary = if let Some(path) = current {
            run.project_path = path;
            if run.project_id.is_some() || database.project_by_path(&run.project_path)?.is_none() {
                if hooks_approved(&database, &run.project_path, &run.approval_commands)? {
                    return Ok(true);
                }
                // A setup's commands come from the config applied at first success, which can
                // differ from what discovery checked. Discovery links a skipped prepare's worktree.
                database.complete_job(
                    &run.job_id,
                    &format!(
                        "{} skipped: hook commands aren't approved; run `pv link` in {} to review them",
                        run.phase, run.project_path
                    ),
                )?;
                self.state().prepares.remove(&run.scope);
                return Ok(false);
            }
            "prepare skipped: linked as an ordinary Project"
        } else {
            "prepare skipped: worktree removed"
        };
        if run.phase == "prepare" {
            self.discard_prepare(&mut self.state(), &mut database, &run.scope, summary)?;
        } else {
            self.record_outcome(run, &Err(HookError::Interrupted));
        }
        Ok(false)
    }

    fn registered_path(
        &self,
        source: &Utf8Path,
        path: &Utf8Path,
        worktree_id: &str,
        scope: &str,
    ) -> Option<Utf8PathBuf> {
        match worktrees::registered_worktree_path(source, path, worktree_id) {
            Ok(registered) => registered,
            Err(error) => {
                // An unreadable Git entry is not evidence of removal. Discovery skips new
                // removal decisions for the same source until it can read the entry.
                structured_log::worktree_discovery_failed(&self.paths, scope, &error.to_string());
                Some(path.to_owned())
            }
        }
    }

    pub(crate) async fn run(
        &self,
        debouncer: ReconciliationDebouncer,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut tasks = JoinSet::new();
        loop {
            while tasks.len() < CONCURRENCY && !*shutdown.borrow() {
                let Some(mut run) = self.state().queued.pop_front() else {
                    break;
                };
                match self.ready_to_run(&mut run) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(error) => {
                        self.complete(
                            &run,
                            &Err(HookError::Io(io::Error::other(error.to_string()))),
                        );
                        continue;
                    }
                }
                let (cancel, receiver) = watch::channel(false);
                self.state().active.insert(
                    run.scope.clone(),
                    ActiveRun {
                        run: run.clone(),
                        cancel,
                    },
                );
                let paths = self.paths.clone();
                tasks.spawn(async move {
                    let outcome = execute(&paths, &run, receiver).await;
                    (run, outcome)
                });
            }
            tokio::select! {
                biased;
                _ = async { let _result = shutdown.wait_for(|requested| *requested).await; } => break,
                joined = tasks.join_next(), if !tasks.is_empty() => {
                    match joined {
                        Some(Ok((run, outcome))) => {
                            self.complete(&run, &outcome);
                            if outcome.is_ok() && run.phase == "prepare" && self.is_prepared(&run.source_project_id, &run.worktree_id) {
                                debouncer.request(crate::ReconciliationScope::System).await;
                            }
                        }
                        Some(Err(error)) => structured_log::job_abandonment_failed(&self.paths, "unknown", "hooks", &error.to_string()),
                        None => {}
                    }
                }
                _ = self.changed.notified() => {}
            }
        }
        self.begin_shutdown();
        let queued = self.state().queued.drain(..).collect::<Vec<_>>();
        for run in queued {
            self.record_outcome(&run, &Err(HookError::Interrupted));
        }
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((run, outcome)) => self.complete(&run, &outcome),
                Err(error) => structured_log::job_abandonment_failed(
                    &self.paths,
                    "unknown",
                    "hooks",
                    &error.to_string(),
                ),
            }
        }
        self.state().active.clear();
        self.changed.notify_waiters();
    }

    fn complete(&self, run: &HookRun, outcome: &Result<(), HookError>) {
        let mut state = self.state();
        state.active.remove(&run.scope);
        self.changed.notify_waiters();
        if run.phase == "prepare" {
            let Some(prepare) = state.prepares.get_mut(&run.scope) else {
                // Its subject is gone. Keep the successful skip even if cleanup failed.
                if let Err(error @ (HookError::Cleanup { .. } | HookError::CleanupFailed(_))) =
                    outcome
                {
                    structured_log::job_failed(
                        &self.paths,
                        &run.job_id,
                        "hooks",
                        &run.scope,
                        &error.to_string(),
                    );
                }
                if let Err(error) = delete_log(&run.log) {
                    structured_log::worktree_cleanup_failed(&self.paths, &error.to_string());
                }
                return;
            };
            prepare.outcome = if outcome.is_ok() {
                PrepareOutcome::Prepared
            } else {
                PrepareOutcome::Failed
            };
        }
        self.record_outcome(run, outcome);
    }

    fn record_outcome(&self, run: &HookRun, outcome: &Result<(), HookError>) {
        let subject = JobDiagnosticSubject::Other {
            kind: "hooks".to_owned(),
            scope: run.scope.clone(),
        };
        let message = match outcome {
            Ok(()) => format!("{} hooks completed (log: {})", run.phase, run.log),
            Err(error) => {
                let retry = if run.phase == "prepare" {
                    "; fix the cause and run `pv daemon:restart`"
                } else {
                    ""
                };
                format!("{}: {error} (log: {}){retry}", run.phase, run.log)
            }
        };
        if outcome.is_err() {
            // Retain the command failure even if the database cannot persist its outcome.
            structured_log::job_failed(&self.paths, &run.job_id, "hooks", &run.scope, &message);
        }
        let recording = Database::open(&self.paths).and_then(|mut database| {
            if outcome.is_ok() {
                database.complete_job_with_coverage(&run.job_id, &message, &[subject])
            } else {
                database.fail_job_with_subject(&run.job_id, &message, &subject)
            }
        });
        match (outcome, recording) {
            (Ok(()), Ok(())) => structured_log::job_completed(
                &self.paths,
                &run.job_id,
                "hooks",
                &run.scope,
                &message,
            ),
            (Ok(()), Err(error)) => structured_log::job_completion_recording_failed(
                &self.paths,
                &run.job_id,
                "hooks",
                &run.scope,
                &message,
                &error.to_string(),
            ),
            (Err(_), Err(error)) => structured_log::job_failure_recording_failed(
                &self.paths,
                &run.job_id,
                "hooks",
                &run.scope,
                &message,
                &error.to_string(),
            ),
            (Err(_), Ok(())) => {}
        }
    }
}

/// Every command a Project's hooks declare, in order.
pub(crate) fn all_hook_commands(hooks: &ProjectHooks) -> Vec<String> {
    hooks.prepare.iter().chain(&hooks.setup).cloned().collect()
}

/// Hooks run only when every one of their commands is approved for the Project's folder.
pub(crate) fn hooks_approved(
    database: &Database,
    project_path: &Utf8Path,
    commands: &[String],
) -> Result<bool, DaemonError> {
    let folder = worktrees::hook_approval_folder(project_path);

    Ok(database
        .unapproved_hook_commands(&folder, commands)?
        .is_empty())
}

fn worktree_scope(source: &str, worktree: &str) -> String {
    format!("worktree:{source}:{worktree}")
}

async fn execute(
    paths: &PvPaths,
    run: &HookRun,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), HookError> {
    structured_log::job_started(paths, &run.job_id, "hooks", &run.scope);
    // Create/truncate the log even if environment capture fails before the first command.
    state::fs::write_sensitive_file(&run.log, "")?;
    let environment = hooks::capture_environment(
        &hooks::login_shell()?,
        paths.home(),
        &run.project_path,
        &mut shutdown,
    )
    .await?;
    hooks::run_commands(
        &HookCommandContext {
            paths,
            project_path: &run.project_path,
            source_path: Some(&run.source_path),
            environment: &environment,
            output: HookOutput::Log(&run.log),
        },
        run.phase,
        &run.commands,
        &mut shutdown,
        |_announcement| Ok(()),
    )
    .await
}

pub(crate) fn delete_log(path: &Utf8Path) -> Result<(), DaemonError> {
    state::fs::remove_file_if_exists(path).map_err(DaemonError::from)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::collections::BTreeSet;
    use std::io;
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use camino_tempfile::tempdir;
    use platform::hooks::HookError;
    use serde_json::Value;
    use state::{
        Database, JobStatus, LinkDiscoveredProjectInput, LinkProjectInput, ProjectMode, PvPaths,
    };
    use tokio::sync::watch;
    use tokio::time::{sleep, timeout};

    use super::{DaemonHooks, HookRun};
    use crate::jobs::DaemonDownloadProgress;
    use crate::project_env::{
        ProjectApplyOptions, ProjectApplyStage,
        reconcile_project_env_with_runtime_catalog_and_progress,
    };
    use crate::reconciliation::ReconciliationDebouncer;

    #[test]
    fn skipped_prepare_cleanup_failure_keeps_its_success_and_logs_the_group() -> Result<()> {
        let directory = tempdir()?;
        let paths = PvPaths::for_home(directory.path());
        let scope = "worktree:project_fixture:tree";
        let mut database = Database::open(&paths)?;
        let job = database.record_finished_job(
            "hooks",
            scope,
            Ok("prepare skipped: worktree removed"),
            &[],
        )?;
        let run = HookRun {
            job_id: job.id.clone(),
            scope: scope.to_owned(),
            phase: "prepare",
            commands: Vec::new(),
            approval_commands: Vec::new(),
            project_path: paths.home().to_owned(),
            source_path: paths.home().to_owned(),
            log: paths.worktree_hook_log("project_fixture", "tree"),
            project_id: None,
            worktree_id: "tree".to_owned(),
            source_project_id: "project_fixture".to_owned(),
        };
        DaemonHooks::new(paths.clone()).complete(
            &run,
            &Err(HookError::CleanupFailed(io::Error::other(
                "hook process group 1234 did not exit",
            ))),
        );
        assert_eq!(database.job(&job.id)?, Some(job));
        let event: Value = serde_json::from_str(&state::fs::read_to_string(&paths.daemon_log())?)?;
        insta::assert_debug_snapshot!((&event["scope"], &event["error"]), @r#"
        (
            String("worktree:project_fixture:tree"),
            String("hook process-group cleanup failed: hook process group 1234 did not exit"),
        )
        "#);
        Ok(())
    }

    #[tokio::test]
    async fn setup_uses_applied_commands_after_the_config_disappears() -> Result<()> {
        let directory = tempdir()?;
        let paths = PvPaths::for_home(directory.path().join("home"));
        let source_path = directory.path().join("source");
        let project_path = directory.path().join("worktree");
        let entry = source_path.join(".git/worktrees/fixture");
        state::fs::write_sensitive_file(&source_path.join("pv.yml"), "serve: false\n")?;
        state::fs::write_sensitive_file(&entry.join("gitdir"), &format!("{project_path}/.git\n"))?;
        state::fs::write_sensitive_file(&entry.join("pv-id"), "fixtureid")?;
        state::fs::write_sensitive_file(&project_path.join(".git"), &format!("gitdir: {entry}\n"))?;
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(
            &config_path,
            "serve: false\nhooks:\n  setup:\n    - touch setup-ran\n",
        )?;
        let mut database = Database::open(&paths)?;
        database.approve_hook_commands(
            &crate::worktrees::hook_approval_folder(&project_path),
            &["touch setup-ran".to_owned()],
        )?;
        let source = database
            .link_project_with_mode(
                LinkProjectInput {
                    path: source_path.clone(),
                    original_path: source_path,
                    primary_hostname: "source.test".to_owned(),
                    config_path: directory.path().join("source/pv.yml"),
                    desired_php_track: None,
                },
                ProjectMode::ResourceOnly,
            )?
            .project;
        let project = database.link_discovered_project(LinkDiscoveredProjectInput {
            source_project_id: source.id,
            worktree_id: "fixtureid".to_owned(),
            worktree_name: "worktree".to_owned(),
            path: project_path.clone(),
            config_path: config_path.clone(),
            mode: ProjectMode::ResourceOnly,
        })?;
        let hooks = DaemonHooks::new(paths.clone());
        reconcile_project_env_with_runtime_catalog_and_progress(
            &paths,
            &project.id,
            None,
            None,
            &BTreeSet::new(),
            DaemonDownloadProgress::disabled(),
            ProjectApplyOptions::new(ProjectApplyStage::CompleteApply, None)
                .with_hooks(Some(&hooks)),
        )
        .await?;
        state::fs::delete_file(&config_path)?;
        let queued = database
            .active_hook_job(&format!("project:{}", project.id))?
            .ok_or_else(|| anyhow!("setup was not queued"))?;
        let (shutdown, receiver) = watch::channel(false);
        let runner = hooks.clone();
        let task = tokio::spawn(async move {
            runner
                .run(
                    ReconciliationDebouncer::new(Duration::ZERO, |_| {}),
                    receiver,
                )
                .await
        });
        let completed = timeout(Duration::from_secs(10), async {
            loop {
                if let Some(job) = database.job(&queued.id)?
                    && job.status != JobStatus::Running
                {
                    return Ok::<_, state::StateError>(job);
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        shutdown.send_replace(true);
        task.await?;
        let completed = completed??;
        anyhow::ensure!(
            completed.status == JobStatus::Succeeded,
            "setup failed: {:?}",
            completed.error
        );
        assert!(state::fs::path_entry_exists(
            &project_path.join("setup-ran")
        )?);
        Ok(())
    }
}
