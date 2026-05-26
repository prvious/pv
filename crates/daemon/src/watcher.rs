use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use camino::Utf8PathBuf;
use state::{Database, PvPaths, fs};
use tokio::time::sleep;

use crate::DaemonError;
use crate::reconciliation::{ReconciliationDebouncer, ReconciliationScope};

pub(crate) struct ProjectConfigWatcher {
    paths: PvPaths,
    debouncer: ReconciliationDebouncer,
    poll_interval: Duration,
    watched_configs: BTreeMap<String, WatchedConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WatchedConfig {
    path: Utf8PathBuf,
    modified_at: Option<SystemTime>,
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
            let modified_at = fs::modified_at(&watch.config_path)?;
            let watched_config = WatchedConfig {
                path: watch.config_path,
                modified_at,
            };

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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use camino_tempfile::tempdir;
    use rusqlite::params;
    use state::{Database, PvPaths, fs};
    use tokio::time::timeout;

    use super::ProjectConfigWatcher;
    use crate::reconciliation::ReconciliationDebouncer;

    #[tokio::test]
    async fn watcher_returns_poll_errors_to_the_task_owner() -> anyhow::Result<()> {
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
    async fn watcher_returns_project_config_modified_at_errors_to_the_task_owner()
    -> anyhow::Result<()> {
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
                    "/tmp/bad",
                    "bad.test",
                    "bad\0path",
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
}
