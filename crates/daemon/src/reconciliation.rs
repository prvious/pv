use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use state::{PvPaths, StateError, UpdateLock};
use thiserror::Error;
use tokio::sync::Notify;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ReconciliationScope {
    System,
    Project {
        id: ReconciliationScopeComponent,
    },
    Resource {
        name: ReconciliationScopeComponent,
        track: ReconciliationScopeComponent,
    },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ReconciliationScopeComponent(String);

#[derive(Clone)]
pub struct ReconciliationQueue {
    inner: Arc<QueueInner>,
}

pub enum EnqueueResult {
    Queued(QueuedReconciliation),
    Coalesced(ReconciliationJob),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationJob {
    scope: ReconciliationScope,
    job_id: String,
}

pub struct QueuedReconciliation {
    job: ReconciliationJob,
    inner: Arc<QueueInner>,
    abandon_job: Option<JobFinalizer>,
    released: bool,
}

pub struct RunningReconciliation {
    job: ReconciliationJob,
    inner: Arc<QueueInner>,
    abandon_job: Option<JobFinalizer>,
    finished: bool,
}

#[derive(Clone)]
pub struct ReconciliationDebouncer {
    delay: Duration,
    state: Arc<DebounceState>,
    handler: Arc<dyn Fn(ReconciliationScope) + Send + Sync>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ReconciliationScopeParseError {
    #[error("unknown reconciliation scope `{scope}`")]
    UnknownScope { scope: String },

    #[error("reconciliation scope `{scope}` has {actual} components; expected {expected}")]
    InvalidComponentCount {
        scope: String,
        expected: usize,
        actual: usize,
    },

    #[error("reconciliation scope `{scope}` has an empty {component} component")]
    EmptyComponent {
        scope: String,
        component: &'static str,
    },
}

struct QueueInner {
    state: Mutex<QueueState>,
    notify: Notify,
}

type JobFinalizer = Box<dyn FnOnce(&str) + Send + 'static>;

struct DebounceState {
    inner: Mutex<DebounceInner>,
}

#[derive(Debug, Default)]
struct QueueState {
    active: Option<ReconciliationJob>,
    queued: VecDeque<ReconciliationJob>,
    update_lock: Option<UpdateLock>,
}

#[derive(Debug, Default)]
struct DebounceInner {
    pending: BTreeSet<ReconciliationScope>,
    worker_running: bool,
}

impl ReconciliationQueue {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(QueueInner {
                state: Mutex::new(QueueState::default()),
                notify: Notify::new(),
            }),
        }
    }

    pub fn enqueue<E>(
        &self,
        scope: ReconciliationScope,
        create_job_id: impl FnOnce() -> Result<String, E>,
    ) -> Result<EnqueueResult, E> {
        self.enqueue_with_abandon(scope, create_job_id, |_job_id| {})
    }

    pub(crate) fn enqueue_with_abandon<E>(
        &self,
        scope: ReconciliationScope,
        create_job_id: impl FnOnce() -> Result<String, E>,
        abandon_job: impl FnOnce(&str) + Send + 'static,
    ) -> Result<EnqueueResult, E> {
        let mut state = lock_queue_state(&self.inner);

        if let Some(job) = matching_queued_job(&state, &scope) {
            return Ok(EnqueueResult::Coalesced(job));
        }

        let job = ReconciliationJob {
            scope,
            job_id: create_job_id()?,
        };
        state.queued.push_back(job.clone());
        self.inner.notify.notify_waiters();

        Ok(EnqueueResult::Queued(QueuedReconciliation {
            job,
            inner: Arc::clone(&self.inner),
            abandon_job: Some(Box::new(abandon_job)),
            released: false,
        }))
    }

    pub(crate) fn enqueue_mutating_with_abandon<E>(
        &self,
        paths: &PvPaths,
        scope: ReconciliationScope,
        create_job_id: impl FnOnce() -> Result<String, E>,
        abandon_job: impl FnOnce(&str) + Send + 'static,
    ) -> Result<EnqueueResult, E>
    where
        E: From<StateError>,
    {
        let mut state = lock_queue_state(&self.inner);

        if let Some(job) = matching_queued_job(&state, &scope) {
            return Ok(EnqueueResult::Coalesced(job));
        }

        let acquired_lock = state.update_lock.is_none();
        if acquired_lock {
            state.update_lock = Some(UpdateLock::acquire(paths).map_err(E::from)?);
        }

        let job_id = match create_job_id() {
            Ok(job_id) => job_id,
            Err(error) => {
                if acquired_lock && state.active.is_none() && state.queued.is_empty() {
                    state.update_lock = None;
                }

                return Err(error);
            }
        };

        let job = ReconciliationJob { scope, job_id };
        state.queued.push_back(job.clone());
        self.inner.notify.notify_waiters();

        Ok(EnqueueResult::Queued(QueuedReconciliation {
            job,
            inner: Arc::clone(&self.inner),
            abandon_job: Some(Box::new(abandon_job)),
            released: false,
        }))
    }
}

impl Default for ReconciliationQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl QueuedReconciliation {
    pub async fn wait_for_turn(self) -> RunningReconciliation {
        let mut queued = self;

        loop {
            let notified = queued.inner.notify.notified();

            {
                let mut state = lock_queue_state(&queued.inner);
                if state.active.is_none()
                    && let Some(front) = state.queued.front()
                    && front.job_id == queued.job.job_id
                    && let Some(job) = state.queued.pop_front()
                {
                    state.active = Some(job.clone());
                    let abandon_job = queued.abandon_job.take();
                    queued.released = true;

                    return RunningReconciliation {
                        job,
                        inner: Arc::clone(&queued.inner),
                        abandon_job,
                        finished: false,
                    };
                }
            }

            notified.await;
        }
    }

    pub fn job_id(&self) -> &str {
        &self.job.job_id
    }
}

impl Drop for QueuedReconciliation {
    fn drop(&mut self) {
        if self.released {
            return;
        }

        if remove_queued_job(&self.inner, &self.job)
            && let Some(abandon_job) = self.abandon_job.take()
        {
            abandon_job(&self.job.job_id);
        }
    }
}

impl RunningReconciliation {
    pub fn scope(&self) -> &ReconciliationScope {
        &self.job.scope
    }

    pub fn job_id(&self) -> &str {
        &self.job.job_id
    }

    pub fn finish(mut self) {
        release_active_job(&self.inner, &self.job);
        self.finished = true;
    }
}

impl Drop for RunningReconciliation {
    fn drop(&mut self) {
        if self.finished {
            return;
        }

        if release_active_job(&self.inner, &self.job)
            && let Some(abandon_job) = self.abandon_job.take()
        {
            abandon_job(&self.job.job_id);
        }
    }
}

impl ReconciliationDebouncer {
    pub fn new(
        delay: Duration,
        handler: impl Fn(ReconciliationScope) + Send + Sync + 'static,
    ) -> Self {
        Self {
            delay,
            state: Arc::new(DebounceState {
                inner: Mutex::new(DebounceInner::default()),
            }),
            handler: Arc::new(handler),
        }
    }

    pub async fn request(&self, scope: ReconciliationScope) {
        let should_spawn = {
            let mut inner = lock_debounce_state(&self.state);
            inner.pending.insert(scope);

            if inner.worker_running {
                false
            } else {
                inner.worker_running = true;
                true
            }
        };

        if should_spawn {
            let debouncer = self.clone();
            tokio::spawn(async move {
                debouncer.flush_after_delay().await;
            });
        }
    }

    async fn flush_after_delay(self) {
        tokio::time::sleep(self.delay).await;

        let scopes = {
            let mut inner = lock_debounce_state(&self.state);
            inner.worker_running = false;
            std::mem::take(&mut inner.pending)
        };

        for scope in scopes {
            (self.handler)(scope);
        }
    }
}

impl ReconciliationJob {
    pub fn scope(&self) -> &ReconciliationScope {
        &self.scope
    }

    pub fn job_id(&self) -> &str {
        &self.job_id
    }
}

impl ReconciliationScope {
    pub fn project(id: impl Into<String>) -> Result<Self, ReconciliationScopeParseError> {
        let id = id.into();
        let scope = format!("project:{id}");
        let id = ReconciliationScopeComponent::new(id, "id", &scope, 2)?;

        Ok(Self::Project { id })
    }

    pub fn resource(
        name: impl Into<String>,
        track: impl Into<String>,
    ) -> Result<Self, ReconciliationScopeParseError> {
        let name = name.into();
        let track = track.into();
        let scope = format!("resource:{name}:{track}");
        let name = ReconciliationScopeComponent::new(name, "name", &scope, 3)?;
        let track = ReconciliationScopeComponent::new(track, "track", &scope, 3)?;

        Ok(Self::Resource { name, track })
    }
}

impl ReconciliationScopeComponent {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn new(
        value: String,
        component: &'static str,
        scope: &str,
        expected_components: usize,
    ) -> Result<Self, ReconciliationScopeParseError> {
        if value.is_empty() {
            return Err(ReconciliationScopeParseError::EmptyComponent {
                scope: scope.to_string(),
                component,
            });
        }

        let actual_components = scope.split(':').count();
        if actual_components != expected_components {
            return Err(ReconciliationScopeParseError::InvalidComponentCount {
                scope: scope.to_string(),
                expected: expected_components,
                actual: actual_components,
            });
        }

        Ok(Self(value))
    }
}

impl fmt::Display for ReconciliationScopeComponent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Display for ReconciliationScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::System => formatter.write_str("system"),
            Self::Project { id } => write!(formatter, "project:{id}"),
            Self::Resource { name, track } => write!(formatter, "resource:{name}:{track}"),
        }
    }
}

impl FromStr for ReconciliationScope {
    type Err = ReconciliationScopeParseError;

    fn from_str(scope: &str) -> Result<Self, Self::Err> {
        let parts = scope.split(':').collect::<Vec<_>>();

        match parts.as_slice() {
            ["system"] => Ok(Self::System),
            ["project", id] if !id.is_empty() => Ok(Self::Project {
                id: ReconciliationScopeComponent((*id).to_string()),
            }),
            ["project", _id] => Err(ReconciliationScopeParseError::EmptyComponent {
                scope: scope.to_string(),
                component: "id",
            }),
            ["project", ..] => Err(ReconciliationScopeParseError::InvalidComponentCount {
                scope: scope.to_string(),
                expected: 2,
                actual: parts.len(),
            }),
            ["resource", name, track] if !name.is_empty() && !track.is_empty() => {
                Ok(Self::Resource {
                    name: ReconciliationScopeComponent((*name).to_string()),
                    track: ReconciliationScopeComponent((*track).to_string()),
                })
            }
            ["resource", "", _track] => Err(ReconciliationScopeParseError::EmptyComponent {
                scope: scope.to_string(),
                component: "name",
            }),
            ["resource", _name, ""] => Err(ReconciliationScopeParseError::EmptyComponent {
                scope: scope.to_string(),
                component: "track",
            }),
            ["resource", ..] => Err(ReconciliationScopeParseError::InvalidComponentCount {
                scope: scope.to_string(),
                expected: 3,
                actual: parts.len(),
            }),
            _ => Err(ReconciliationScopeParseError::UnknownScope {
                scope: scope.to_string(),
            }),
        }
    }
}

fn matching_queued_job(
    state: &QueueState,
    scope: &ReconciliationScope,
) -> Option<ReconciliationJob> {
    state.queued.iter().find(|job| &job.scope == scope).cloned()
}

fn remove_queued_job(inner: &QueueInner, job: &ReconciliationJob) -> bool {
    let mut state = lock_queue_state(inner);
    let queued_len = state.queued.len();
    state.queued.retain(|queued| queued.job_id != job.job_id);

    if state.queued.len() != queued_len {
        release_update_lock_if_idle(&mut state);
        inner.notify.notify_waiters();
        return true;
    }

    false
}

fn release_active_job(inner: &QueueInner, job: &ReconciliationJob) -> bool {
    let mut state = lock_queue_state(inner);

    if state
        .active
        .as_ref()
        .is_some_and(|active| active.job_id == job.job_id)
    {
        state.active = None;
        release_update_lock_if_idle(&mut state);
        inner.notify.notify_waiters();
        return true;
    }

    false
}

fn release_update_lock_if_idle(state: &mut QueueState) {
    if state.active.is_none() && state.queued.is_empty() {
        state.update_lock = None;
    }
}

fn lock_queue_state(inner: &QueueInner) -> MutexGuard<'_, QueueState> {
    match inner.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn lock_debounce_state(state: &DebounceState) -> MutexGuard<'_, DebounceInner> {
    match state.inner.lock() {
        Ok(inner) => inner,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::{Duration, sleep, timeout};

    use super::{
        EnqueueResult, QueuedReconciliation, ReconciliationDebouncer, ReconciliationQueue,
        ReconciliationScope, ReconciliationScopeParseError,
    };

    #[tokio::test]
    async fn queue_runs_one_scope_at_a_time_and_coalesces_duplicates() -> anyhow::Result<()> {
        let queue = ReconciliationQueue::new();
        let first = queued(queue.enqueue(ReconciliationScope::System, || {
            Ok::<String, anyhow::Error>("job_1".to_string())
        })?)?;
        let first_run = first.wait_for_turn().await;
        assert_eq!(first_run.job_id(), "job_1");

        let duplicate = queue.enqueue(ReconciliationScope::System, || {
            Ok::<String, anyhow::Error>("job_duplicate".to_string())
        })?;
        let trailing = queued(duplicate)?;
        assert_eq!(trailing.job_id(), "job_duplicate");

        let project_scope = ReconciliationScope::project("project_1")?;
        let project = queued(queue.enqueue(project_scope.clone(), || {
            Ok::<String, anyhow::Error>("job_2".to_string())
        })?)?;
        let project_wait = timeout(Duration::from_millis(10), project.wait_for_turn()).await;
        assert!(project_wait.is_err());

        first_run.finish();

        let trailing_run = timeout(Duration::from_secs(1), trailing.wait_for_turn()).await?;
        assert_eq!(trailing_run.scope(), &ReconciliationScope::System);
        trailing_run.finish();

        let project = queued(queue.enqueue(project_scope.clone(), || {
            Ok::<String, anyhow::Error>("job_3".to_string())
        })?)?;
        let project_run = timeout(Duration::from_secs(1), project.wait_for_turn()).await?;
        assert_eq!(project_run.scope(), &project_scope);
        assert_eq!(project_run.job_id(), "job_3");
        project_run.finish();

        Ok(())
    }

    #[tokio::test]
    async fn dropping_running_reconciliation_releases_the_active_scope() -> anyhow::Result<()> {
        let queue = ReconciliationQueue::new();
        let running = queued(queue.enqueue(ReconciliationScope::System, || {
            Ok::<String, anyhow::Error>("job_1".to_string())
        })?)?
        .wait_for_turn()
        .await;

        drop(running);

        assert!(matches!(
            queue.enqueue(ReconciliationScope::System, || {
                Ok::<String, anyhow::Error>("job_2".to_string())
            })?,
            EnqueueResult::Queued(_)
        ));

        Ok(())
    }

    #[tokio::test]
    async fn duplicate_active_scope_runs_one_trailing_reconciliation() -> anyhow::Result<()> {
        let queue = ReconciliationQueue::new();
        let active = queued(queue.enqueue(ReconciliationScope::System, || {
            Ok::<String, anyhow::Error>("job_1".to_string())
        })?)?
        .wait_for_turn()
        .await;

        let trailing = queued(queue.enqueue(ReconciliationScope::System, || {
            Ok::<String, anyhow::Error>("job_2".to_string())
        })?)?;
        let duplicate_trailing = queue.enqueue(ReconciliationScope::System, || {
            Ok::<String, anyhow::Error>("job_3".to_string())
        })?;

        assert!(matches!(
            duplicate_trailing,
            EnqueueResult::Coalesced(job) if job.job_id() == "job_2"
        ));

        active.finish();

        let running = timeout(Duration::from_secs(1), trailing.wait_for_turn()).await?;
        assert_eq!(running.job_id(), "job_2");
        running.finish();

        Ok(())
    }

    #[tokio::test]
    async fn debounce_coalesces_bursts_before_queueing_reconciliation() -> anyhow::Result<()> {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let debouncer = ReconciliationDebouncer::new(Duration::from_millis(10), move |scope| {
            let _sent = sender.send(scope);
        });
        let project_scope = ReconciliationScope::project("project_1")?;

        debouncer.request(ReconciliationScope::System).await;
        debouncer.request(ReconciliationScope::System).await;
        debouncer.request(project_scope.clone()).await;
        sleep(Duration::from_millis(50)).await;

        let mut scopes = Vec::new();
        while let Ok(scope) = receiver.try_recv() {
            scopes.push(scope);
        }

        scopes.sort();
        assert_eq!(scopes, vec![ReconciliationScope::System, project_scope]);

        Ok(())
    }

    #[test]
    fn scope_parser_rejects_extra_or_missing_components_with_typed_errors() {
        assert!(matches!(
            "resource:mysql:8.4".parse::<ReconciliationScope>(),
            Ok(ReconciliationScope::Resource { name, track })
                if name.as_str() == "mysql" && track.as_str() == "8.4"
        ));
        assert!(matches!(
            "resource:mysql:8.4:extra".parse::<ReconciliationScope>(),
            Err(ReconciliationScopeParseError::InvalidComponentCount {
                scope,
                expected: 3,
                actual: 4,
            }) if scope == "resource:mysql:8.4:extra"
        ));
        assert!(matches!(
            "project:".parse::<ReconciliationScope>(),
            Err(ReconciliationScopeParseError::EmptyComponent {
                scope,
                component: "id",
            }) if scope == "project:"
        ));
        assert!(matches!(
            "unknown:scope".parse::<ReconciliationScope>(),
            Err(ReconciliationScopeParseError::UnknownScope { scope })
                if scope == "unknown:scope"
        ));
    }

    #[test]
    fn scope_constructors_reject_ambiguous_components() {
        assert!(matches!(
            ReconciliationScope::project(""),
            Err(ReconciliationScopeParseError::EmptyComponent {
                component: "id",
                ..
            })
        ));
        assert!(matches!(
            ReconciliationScope::project("project:one"),
            Err(ReconciliationScopeParseError::InvalidComponentCount { .. })
        ));
        assert!(matches!(
            ReconciliationScope::resource("mysql", "8.4:debug"),
            Err(ReconciliationScopeParseError::InvalidComponentCount { .. })
        ));
    }

    fn queued(result: EnqueueResult) -> anyhow::Result<QueuedReconciliation> {
        match result {
            EnqueueResult::Queued(queued) => Ok(queued),
            EnqueueResult::Coalesced(job) => Err(anyhow::anyhow!(
                "scope unexpectedly coalesced into {}",
                job.job_id()
            )),
        }
    }
}
