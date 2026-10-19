//! Daemon hook runs have their own FIFO and never enter the reconciliation queue.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use platform::hooks::{self, HookCommandContext, HookError, HookOutput};
use state::{Database, JobDiagnosticSubject, PvPaths};
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;
use tokio::time::{MissedTickBehavior, interval};

use crate::DaemonError;
use crate::reconciliation::ReconciliationDebouncer;
use crate::{structured_log, worktrees};

const CONCURRENCY: usize = 4;
const OBSOLETE_SCAN_INTERVAL: Duration = Duration::from_millis(100);

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
    cleanup_failures: BTreeMap<String, String>,
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

    /// An ordinary `pv link` takes ownership of this path. An already executing prepare may
    /// finish, but its result must never put the path back into discovery's transient sets.
    pub(crate) fn forget_linked_prepares(&self, database: &Database) -> Result<(), DaemonError> {
        let projects = database.projects()?;
        self.state().prepares.retain(|scope, prepare| {
            if projects.iter().any(|project| project.path == prepare.path) {
                return false;
            }
            let current = projects
                .iter()
                .find(|project| project.id == prepare.source_project_id)
                .and_then(|source| {
                    self.registered_path(&source.path, &prepare.path, &prepare.worktree_id, scope)
                });
            if let Some(current) = current {
                prepare.path = current;
            }
            !projects.iter().any(|project| project.path == prepare.path)
        });
        Ok(())
    }

    pub(crate) fn queue_prepare(
        &self,
        source_project_id: &str,
        worktree_id: &str,
        project_path: &Utf8Path,
        commands: Vec<String>,
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
            commands,
            project_path: project_path.to_owned(),
            source_path: source.path,
            log,
            project_id: None,
            worktree_id: worktree_id.to_owned(),
            source_project_id: source_project_id.to_owned(),
        };
        if state.stopping {
            self.record_outcome(&run, &Err(HookError::Interrupted))?;
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
        commands: Vec<String>,
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
        if state
            .active
            .values()
            .any(|active| active.run.job_id == job_id)
            || state.queued.iter().any(|run| run.job_id == job_id)
        {
            return Ok(());
        }
        let run = HookRun {
            job_id: job_id.to_string(),
            scope,
            phase: "setup",
            commands,
            project_path: project.path,
            source_path: source.path,
            log,
            project_id: Some(project_id.to_owned()),
            worktree_id: discovered.worktree_id,
            source_project_id: discovered.source_project_id,
        };
        if state.stopping {
            self.record_outcome(&run, &Err(HookError::Interrupted))?;
        } else {
            state.queued.push_back(run);
        }
        self.changed.notify_one();
        Ok(())
    }

    /// Cleanup waits for cancellation and process-group reaping before deleting allocations.
    pub(crate) async fn stop_obsolete(&self) -> Result<(), DaemonError> {
        loop {
            let notified = self.changed.notified();
            let pending = self.cancel_obsolete()?;
            if !pending {
                return Ok(());
            }
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(OBSOLETE_SCAN_INTERVAL) => {}
            }
        }
    }

    fn cancel_obsolete(&self) -> Result<bool, DaemonError> {
        {
            let state = self.state();
            if let Some((scope, reason)) = state.cleanup_failures.iter().next() {
                return Err(DaemonError::HookCleanup {
                    scope: scope.clone(),
                    reason: reason.clone(),
                });
            }
            if state.queued.is_empty() && state.active.is_empty() && state.prepares.is_empty() {
                return Ok(false);
            }
        }
        let database = Database::open(&self.paths)?;
        self.forget_linked_prepares(&database)?;
        let mut state = self.state();
        let mut obsolete = Vec::new();
        for (scope, prepare) in &state.prepares {
            let registered =
                if let Some(source) = database.project_by_id(&prepare.source_project_id)? {
                    self.registered_path(&source.path, &prepare.path, &prepare.worktree_id, scope)
                        .is_some()
                } else {
                    false
                };
            if !registered {
                obsolete.push(scope.clone());
            }
        }
        let HookState { queued, active, .. } = &mut *state;
        for run in queued
            .iter_mut()
            .chain(active.values_mut().map(|active| &mut active.run))
        {
            let project_removed = if let Some(project_id) = &run.project_id {
                if let Some(project) = database.project_by_id(project_id)? {
                    run.project_path = project.path;
                    false
                } else {
                    true
                }
            } else {
                false
            };
            let source = database.project_by_id(&run.source_project_id)?;
            if let Some(source) = &source {
                run.source_path = source.path.clone();
            }
            let registered = self.registered_path(
                &run.source_path,
                &run.project_path,
                &run.worktree_id,
                &run.scope,
            );
            if let Some(path) = &registered {
                run.project_path = path.clone();
            }
            if project_removed || source.is_none() || registered.is_none() {
                obsolete.push(run.scope.clone());
            }
        }
        obsolete.sort();
        obsolete.dedup();
        let mut waiting = false;
        for scope in obsolete {
            if let Some(error) = state.cleanup_failures.get(&scope) {
                return Err(DaemonError::HookCleanup {
                    scope,
                    reason: error.clone(),
                });
            }
            if let Some(active) = state.active.get(&scope) {
                active.cancel.send_replace(true);
                waiting = true;
                continue;
            }
            if let Some(index) = state.queued.iter().position(|run| run.scope == scope)
                && let Some(run) = state.queued.remove(index)
            {
                self.record_outcome(&run, &Err(HookError::Interrupted))?;
            }
            state.prepares.remove(&scope);
            if scope.starts_with("worktree:")
                && let Some(path) = self.paths.hook_log_for_scope(&scope)
            {
                delete_log(&path)?;
            }
        }
        Ok(waiting)
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
    ) -> Result<(), DaemonError> {
        let mut tasks = JoinSet::new();
        let mut result = self.drive(&mut tasks, debouncer, &mut shutdown).await;
        // Recording and discovery errors must drain children just like normal shutdown.
        self.begin_shutdown();
        let queued = self.state().queued.drain(..).collect::<Vec<_>>();
        for run in queued {
            result = combine_results(
                result,
                self.record_outcome(&run, &Err(HookError::Interrupted)),
            );
        }
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((run, outcome)) => {
                    result = combine_results(result, self.complete(&run, &outcome))
                }
                Err(error) => result = combine_results(result, Err(error.into())),
            }
        }
        {
            let mut state = self.state();
            let scopes = state.active.keys().cloned().collect::<Vec<_>>();
            for scope in scopes {
                state.cleanup_failures.insert(
                    scope,
                    "hook task exited before process cleanup was verified".to_owned(),
                );
            }
            state.active.clear();
            if let Some((scope, reason)) = state.cleanup_failures.iter().next() {
                result = combine_results(
                    result,
                    Err(DaemonError::HookCleanup {
                        scope: scope.clone(),
                        reason: reason.clone(),
                    }),
                );
            }
        }
        self.changed.notify_waiters();
        result
    }

    async fn drive(
        &self,
        tasks: &mut JoinSet<(HookRun, Result<(), HookError>)>,
        debouncer: ReconciliationDebouncer,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<(), DaemonError> {
        let mut interval = interval(OBSOLETE_SCAN_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            self.cancel_obsolete()?;
            while tasks.len() < CONCURRENCY && !*shutdown.borrow() {
                let Some(mut run) = self.state().queued.pop_front() else {
                    break;
                };
                // A queued prepare superseded by an ordinary link has nothing left to do.
                if run.phase == "prepare" && !self.state().prepares.contains_key(&run.scope) {
                    self.record_outcome(&run, &Err(HookError::Interrupted))?;
                    continue;
                }
                let database = Database::open(&self.paths)?;
                let source = database.project_by_id(&run.source_project_id)?;
                let current = source.as_ref().and_then(|source| {
                    self.registered_path(
                        &source.path,
                        &run.project_path,
                        &run.worktree_id,
                        &run.scope,
                    )
                });
                let project_linked = if let Some(project_id) = &run.project_id {
                    database.project_by_id(project_id)?.is_some()
                } else {
                    true
                };
                if let (Some(source), Some(path)) = (source, current)
                    && project_linked
                {
                    run.source_path = source.path;
                    run.project_path = path;
                } else {
                    self.record_outcome(&run, &Err(HookError::Interrupted))?;
                    self.state().prepares.remove(&run.scope);
                    continue;
                }
                if let Some(prepare) = self.state().prepares.get_mut(&run.scope) {
                    prepare.path = run.project_path.clone();
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
                    if let Some(joined) = joined {
                        let (run, outcome) = joined?;
                        self.complete(&run, &outcome)?;
                        if outcome.is_ok() && run.phase == "prepare" && self.is_prepared(&run.source_project_id, &run.worktree_id) {
                            debouncer.request(crate::ReconciliationScope::System).await;
                        }
                    }
                }
                _ = self.changed.notified() => {}
                _ = interval.tick() => {}
            }
        }
        Ok(())
    }

    fn complete(&self, run: &HookRun, outcome: &Result<(), HookError>) -> Result<(), DaemonError> {
        {
            let mut state = self.state();
            state.active.remove(&run.scope);
            if let Err(error @ (HookError::Cleanup { .. } | HookError::CleanupFailed(_))) = outcome
            {
                state
                    .cleanup_failures
                    .insert(run.scope.clone(), error.to_string());
            }
        }
        self.changed.notify_waiters();
        let recorded = self.record_outcome(run, outcome);
        let forgotten = Database::open(&self.paths)
            .map_err(DaemonError::from)
            .and_then(|database| self.forget_linked_prepares(&database));
        combine_results(recorded, forgotten)?;
        let mut state = self.state();
        if let Some(prepare) = state.prepares.get_mut(&run.scope) {
            prepare.outcome = if outcome.is_ok() {
                PrepareOutcome::Prepared
            } else {
                PrepareOutcome::Failed
            };
        }
        Ok(())
    }

    fn record_outcome(
        &self,
        run: &HookRun,
        outcome: &Result<(), HookError>,
    ) -> Result<(), DaemonError> {
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
        if let Err(recording) = recording {
            return Err(DaemonError::HookOutcomeRecording {
                outcome: message,
                recording: Box::new(recording),
            });
        }
        if outcome.is_ok() {
            structured_log::job_completed(&self.paths, &run.job_id, "hooks", &run.scope, &message);
        }
        Ok(())
    }
}

fn combine_results(
    original: Result<(), DaemonError>,
    cleanup: Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    match (original, cleanup) {
        (Ok(()), result) | (result, Ok(())) => result,
        (Err(source), Err(cleanup)) => Err(DaemonError::RuntimeCleanupFailed {
            runtime: "Project hooks".to_owned(),
            source: Box::new(source),
            cleanup: Box::new(cleanup),
        }),
    }
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
    match state::fs::delete_file(path) {
        Ok(()) => Ok(()),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use camino_tempfile::tempdir;
    use state::{
        Database, JobStatus, LinkDiscoveredProjectInput, LinkProjectInput, ProjectMode, PvPaths,
    };
    use tokio::sync::watch;
    use tokio::time::{sleep, timeout};

    use super::DaemonHooks;
    use crate::jobs::DaemonDownloadProgress;
    use crate::project_env::{
        ProjectApplyOptions, ProjectApplyStage,
        reconcile_project_env_with_runtime_catalog_and_progress,
    };
    use crate::reconciliation::ReconciliationDebouncer;
    use std::collections::BTreeSet;

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
        task.await??;
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
