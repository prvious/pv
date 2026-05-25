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

    pub(crate) async fn run(mut self) {
        loop {
            let _result = self.poll_once().await;
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
            {
                self.debouncer
                    .request(ReconciliationScope::Project {
                        id: watch.project_id.clone(),
                    })
                    .await;
            }

            current_configs.insert(watch.project_id, watched_config);
        }

        self.watched_configs = current_configs;

        Ok(())
    }
}
