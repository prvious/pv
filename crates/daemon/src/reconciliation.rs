use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, Notify};

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
    Coalesced,
}

#[derive(Clone)]
pub struct QueuedReconciliation {
    scope: ReconciliationScope,
    inner: Arc<QueueInner>,
}

pub struct RunningReconciliation {
    scope: ReconciliationScope,
    inner: Arc<QueueInner>,
}

#[derive(Clone)]
pub struct ReconciliationDebouncer {
    queue: ReconciliationQueue,
    delay: Duration,
    state: Arc<DebounceState>,
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
    active: Option<ReconciliationScope>,
    queued: VecDeque<ReconciliationScope>,
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

    pub async fn enqueue(&self, scope: ReconciliationScope) -> EnqueueResult {
        let mut state = self.inner.state.lock().await;

        if state.active.as_ref() == Some(&scope) || state.queued.contains(&scope) {
            return EnqueueResult::Coalesced;
        }

        state.queued.push_back(scope.clone());
        self.inner.notify.notify_waiters();

        EnqueueResult::Queued(QueuedReconciliation {
            scope,
            inner: Arc::clone(&self.inner),
        })
    }
}

impl Default for ReconciliationQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl QueuedReconciliation {
    pub async fn wait_for_turn(self) -> RunningReconciliation {
        loop {
            let notified = self.inner.notify.notified();

            {
                let mut state = self.inner.state.lock().await;
                if state.active.is_none() && state.queued.front() == Some(&self.scope) {
                    state.queued.pop_front();
                    state.active = Some(self.scope.clone());

                    return RunningReconciliation {
                        scope: self.scope,
                        inner: Arc::clone(&self.inner),
                    };
                }
            }

            notified.await;
        }
    }
}

impl RunningReconciliation {
    pub fn scope(&self) -> &ReconciliationScope {
        &self.scope
    }

    pub async fn finish(self) {
        let mut state = self.inner.state.lock().await;

        if state.active.as_ref() == Some(&self.scope) {
            state.active = None;
            self.inner.notify.notify_waiters();
        }
    }
}

impl ReconciliationDebouncer {
    pub fn new(queue: ReconciliationQueue, delay: Duration) -> Self {
        Self {
            queue,
            delay,
            state: Arc::new(DebounceState {
                inner: Mutex::new(DebounceInner::default()),
            }),
        }
    }

    pub async fn request(&self, scope: ReconciliationScope) {
        let should_spawn = {
            let mut inner = self.state.inner.lock().await;
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
            let mut inner = self.state.inner.lock().await;
            inner.worker_running = false;
            std::mem::take(&mut inner.pending)
        };

        for scope in scopes {
            let _result = self.queue.enqueue(scope).await;
        }
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
    type Err = ();

    fn from_str(scope: &str) -> Result<Self, Self::Err> {
        if scope == "system" {
            return Ok(Self::System);
        }

        if let Some(id) = scope.strip_prefix("project:")
            && !id.is_empty()
        {
            return Ok(Self::Project { id: id.to_string() });
        }

        if let Some(resource) = scope.strip_prefix("resource:") {
            let Some((name, track)) = resource.split_once(':') else {
                return Err(());
            };

            if !name.is_empty() && !track.is_empty() {
                return Ok(Self::Resource {
                    name: name.to_string(),
                    track: track.to_string(),
                });
            }
        }

        Err(())
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::{Duration, sleep, timeout};

    use super::{
        EnqueueResult, QueuedReconciliation, ReconciliationDebouncer, ReconciliationQueue,
        ReconciliationScope,
    };

    #[tokio::test]
    async fn queue_runs_one_scope_at_a_time_and_coalesces_duplicates() -> anyhow::Result<()> {
        let queue = ReconciliationQueue::new();
        let first = queued(queue.enqueue(ReconciliationScope::System).await)?;
        let first_run = first.wait_for_turn().await;

        let duplicate = queue.enqueue(ReconciliationScope::System).await;
        assert!(matches!(duplicate, EnqueueResult::Coalesced));

        let project_scope = ReconciliationScope::Project {
            id: "project_1".to_string(),
        };
        let project = queued(queue.enqueue(project_scope.clone()).await)?;
        let project_wait =
            timeout(Duration::from_millis(10), project.clone().wait_for_turn()).await;
        assert!(project_wait.is_err());

        first_run.finish().await;

        let project_run = timeout(Duration::from_secs(1), project.wait_for_turn()).await?;
        assert_eq!(project_run.scope(), &project_scope);
        project_run.finish().await;

        Ok(())
    }

    #[tokio::test]
    async fn debounce_coalesces_bursts_before_queueing_reconciliation() -> anyhow::Result<()> {
        let queue = ReconciliationQueue::new();
        let debouncer = ReconciliationDebouncer::new(queue.clone(), Duration::from_millis(10));
        let project_scope = ReconciliationScope::Project {
            id: "project_1".to_string(),
        };

        debouncer.request(ReconciliationScope::System).await;
        debouncer.request(ReconciliationScope::System).await;
        debouncer.request(project_scope.clone()).await;
        sleep(Duration::from_millis(50)).await;

        assert!(matches!(
            queue.enqueue(ReconciliationScope::System).await,
            EnqueueResult::Coalesced
        ));
        assert!(matches!(
            queue.enqueue(project_scope).await,
            EnqueueResult::Coalesced
        ));

        Ok(())
    }

    fn queued(result: EnqueueResult) -> anyhow::Result<QueuedReconciliation> {
        match result {
            EnqueueResult::Queued(queued) => Ok(queued),
            EnqueueResult::Coalesced => Err(anyhow::anyhow!("scope unexpectedly coalesced")),
        }
    }
}
