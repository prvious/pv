use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use camino::Utf8Path;
use state::{Database, PvPaths, fs};
use tokio::time::sleep;

use crate::DaemonError;
use crate::reconciliation::{ReconciliationDebouncer, ReconciliationScope};

const PREFERRED_CONFIG_FILE: &str = "pv.yml";
const ALTERNATE_CONFIG_FILE: &str = "pv.yaml";

pub(crate) struct ProjectConfigWatcher {
    paths: PvPaths,
    debouncer: ReconciliationDebouncer,
    poll_interval: Duration,
    watched_configs: BTreeMap<String, WatchedConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WatchedConfig {
    preferred_modified_at: Option<SystemTime>,
    alternate_modified_at: Option<SystemTime>,
}

impl ProjectConfigWatcher {
    pub(crate) fn new(
        paths: PvPaths,
        debouncer: ReconciliationDebouncer,
        poll_interval: Duration,
    ) -> Self {
        Self {
            paths,
            debouncer,
            poll_interval,
            watched_configs: BTreeMap::new(),
        }
    }

    pub(crate) async fn run(mut self) -> Result<(), DaemonError> {
        loop {
            self.poll_once().await?;
            sleep(self.poll_interval).await;
        }
    }

    async fn poll_once(&mut self) -> Result<(), DaemonError> {
        let database = Database::open(&self.paths)?;
        let watches = database.project_config_watches()?;
        let mut current_configs = BTreeMap::new();

        for watch in watches {
            let watched_config = project_config_snapshot(&watch.project_path)?;

            if let Some(previous_config) = self
                .watched_configs
                .insert(watch.project_id.clone(), watched_config.clone())
                && previous_config != watched_config
                && let Ok(scope) = ReconciliationScope::project(watch.project_id.clone())
            {
                self.debouncer.request(scope).await;
            }

            current_configs.insert(watch.project_id, watched_config);
        }

        self.watched_configs = current_configs;

        Ok(())
    }
}

fn project_config_snapshot(project_path: &Utf8Path) -> Result<WatchedConfig, DaemonError> {
    Ok(WatchedConfig {
        preferred_modified_at: fs::modified_at(&project_path.join(PREFERRED_CONFIG_FILE))?,
        alternate_modified_at: fs::modified_at(&project_path.join(ALTERNATE_CONFIG_FILE))?,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use camino_tempfile::tempdir;
    use rusqlite::params;
    use state::{Database, PvPaths, fs};
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
    use tokio::time::timeout;

    use super::ProjectConfigWatcher;
    use crate::reconciliation::ReconciliationDebouncer;

    #[tokio::test]
    async fn watcher_returns_poll_errors_to_the_task_owner() -> Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        fs::write_sensitive_file(paths.root(), "not a directory")?;
        let debouncer = ReconciliationDebouncer::new(Duration::from_millis(1), |_scope| {});
        let watcher = ProjectConfigWatcher::new(paths, debouncer, Duration::from_millis(1));

        let result = timeout(Duration::from_millis(50), watcher.run()).await?;

        assert!(result.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn watcher_returns_project_config_modified_at_errors_to_the_task_owner() -> Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        fs::write_sensitive_file(&config_path, "php: '8.3'\n")?;
        let mut database = Database::open(&paths)?;
        state::testing::transaction(&mut database, |transaction| {
            transaction.execute(
                "INSERT INTO projects (id, path, primary_hostname, config_path, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "bad_project",
                    "bad\0path",
                    "bad.test",
                    "/tmp/bad/pv.yml",
                    "2026-05-25T00:00:00Z",
                    "2026-05-25T00:00:00Z",
                ],
            )?;
            transaction.execute(
                "INSERT INTO projects (id, path, primary_hostname, config_path, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "good_project",
                    project_path.as_str(),
                    "good.test",
                    config_path.as_str(),
                    "2026-05-25T00:00:00Z",
                    "2026-05-25T00:00:00Z",
                ],
            )?;

            Ok(())
        })?;
        let debouncer = ReconciliationDebouncer::new(Duration::from_millis(1), |_scope| {});
        let mut watcher = ProjectConfigWatcher::new(paths, debouncer, Duration::from_millis(1));

        let result = watcher.poll_once().await;

        assert!(matches!(
            result,
            Err(crate::DaemonError::State(state::StateError::Filesystem { source, .. }))
                if source.kind() == std::io::ErrorKind::InvalidInput
        ));

        Ok(())
    }

    #[tokio::test]
    async fn watcher_enqueues_when_preferred_config_appears_after_empty_link() -> Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        insert_project(&paths, &project_path, &config_path)?;
        let (debouncer, mut scopes) = project_scope_recorder();
        let mut watcher = ProjectConfigWatcher::new(paths, debouncer, Duration::from_millis(1));

        watcher.poll_once().await?;
        fs::write_sensitive_file(&config_path, "php: '8.4'\n")?;
        watcher.poll_once().await?;

        assert_eq!(next_scope(&mut scopes).await?, "project:project_1");

        Ok(())
    }

    #[tokio::test]
    async fn watcher_enqueues_when_alternate_config_appears_after_empty_link() -> Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        insert_project(&paths, &project_path, &config_path)?;
        let (debouncer, mut scopes) = project_scope_recorder();
        let mut watcher = ProjectConfigWatcher::new(paths, debouncer, Duration::from_millis(1));

        watcher.poll_once().await?;
        fs::write_sensitive_file(&project_path.join("pv.yaml"), "php: '8.4'\n")?;
        watcher.poll_once().await?;

        assert_eq!(next_scope(&mut scopes).await?, "project:project_1");

        Ok(())
    }

    #[tokio::test]
    async fn watcher_enqueues_when_project_config_conflict_appears() -> Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        fs::write_sensitive_file(&config_path, "php: '8.3'\n")?;
        insert_project(&paths, &project_path, &config_path)?;
        let (debouncer, mut scopes) = project_scope_recorder();
        let mut watcher = ProjectConfigWatcher::new(paths, debouncer, Duration::from_millis(1));

        watcher.poll_once().await?;
        fs::write_sensitive_file(&project_path.join("pv.yaml"), "php: '8.4'\n")?;
        watcher.poll_once().await?;

        assert_eq!(next_scope(&mut scopes).await?, "project:project_1");

        Ok(())
    }

    fn project_scope_recorder() -> (ReconciliationDebouncer, UnboundedReceiver<String>) {
        let (sender, receiver) = unbounded_channel();
        let debouncer = ReconciliationDebouncer::new(Duration::ZERO, move |scope| {
            let _result = sender.send(scope.to_string());
        });

        (debouncer, receiver)
    }

    fn insert_project(
        paths: &PvPaths,
        project_path: &camino::Utf8Path,
        config_path: &camino::Utf8Path,
    ) -> Result<()> {
        let mut database = Database::open(paths)?;
        state::testing::transaction(&mut database, |transaction| {
            transaction.execute(
                "INSERT INTO projects (id, path, primary_hostname, config_path, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "project_1",
                    project_path.as_str(),
                    "project.test",
                    config_path.as_str(),
                    "2026-05-25T00:00:00Z",
                    "2026-05-25T00:00:00Z",
                ],
            )?;

            Ok(())
        })?;

        Ok(())
    }

    async fn next_scope(scopes: &mut UnboundedReceiver<String>) -> Result<String> {
        timeout(Duration::from_millis(100), scopes.recv())
            .await?
            .ok_or_else(|| anyhow!("watcher scope recorder closed"))
    }
}
