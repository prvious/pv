use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use thiserror::Error;
use tokio::sync::Notify;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ReconciliationScope {
    System,
    Project { id: String },
    Resource { name: String, track: String },
}

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
    released: bool,
}

pub struct RunningReconciliation {
    job: ReconciliationJob,
    inner: Arc<QueueInner>,
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

struct DebounceState {
    inner: Mutex<DebounceInner>,
}

#[derive(Debug, Default)]
struct QueueState {
    active: Option<ReconciliationJob>,
    queued: VecDeque<ReconciliationJob>,
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
        let mut state = lock_queue_state(&self.inner);

        if let Some(job) = matching_job(&state, &scope) {
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
                    queued.released = true;

                    return RunningReconciliation {
                        job,
                        inner: Arc::clone(&queued.inner),
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

        remove_queued_job(&self.inner, &self.job);
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

        release_active_job(&self.inner, &self.job);
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
            ["project", id] if !id.is_empty() => Ok(Self::Project { id: id.to_string() }),
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
                    name: name.to_string(),
                    track: track.to_string(),
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

fn matching_job(state: &QueueState, scope: &ReconciliationScope) -> Option<ReconciliationJob> {
    state
        .active
        .iter()
        .chain(state.queued.iter())
        .find(|job| &job.scope == scope)
        .cloned()
}

fn remove_queued_job(inner: &QueueInner, job: &ReconciliationJob) {
    let mut state = lock_queue_state(inner);
    let queued_len = state.queued.len();
    state.queued.retain(|queued| queued.job_id != job.job_id);

    if state.queued.len() != queued_len {
        inner.notify.notify_waiters();
    }
}

fn release_active_job(inner: &QueueInner, job: &ReconciliationJob) {
    let mut state = lock_queue_state(inner);

    if state
        .active
        .as_ref()
        .is_some_and(|active| active.job_id == job.job_id)
    {
        state.active = None;
        inner.notify.notify_waiters();
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
        assert!(matches!(
            duplicate,
            EnqueueResult::Coalesced(job) if job.job_id() == "job_1"
        ));

        let project_scope = ReconciliationScope::Project {
            id: "project_1".to_string(),
        };
        let project = queued(queue.enqueue(project_scope.clone(), || {
            Ok::<String, anyhow::Error>("job_2".to_string())
        })?)?;
        let project_wait = timeout(Duration::from_millis(10), project.wait_for_turn()).await;
        assert!(project_wait.is_err());

        first_run.finish();

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
    async fn debounce_coalesces_bursts_before_queueing_reconciliation() -> anyhow::Result<()> {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let debouncer = ReconciliationDebouncer::new(Duration::from_millis(10), move |scope| {
            let _sent = sender.send(scope);
        });
        let project_scope = ReconciliationScope::Project {
            id: "project_1".to_string(),
        };

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
                if name == "mysql" && track == "8.4"
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
