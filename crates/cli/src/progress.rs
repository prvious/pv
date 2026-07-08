use std::cell::RefCell;
use std::collections::BTreeMap;

use daemon::{JobDownloadProgress, JobEventHandler};
use indicatif::{ProgressBar, ProgressStyle};
use resources::{DownloadProgress, DownloadProgressEvent, ManifestArtifact};

#[derive(Debug)]
pub(crate) struct DownloadProgressRenderer {
    enabled: bool,
    bars: RefCell<BTreeMap<String, ProgressBar>>,
}

impl DownloadProgressRenderer {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            bars: RefCell::new(BTreeMap::new()),
        }
    }

    pub(crate) fn update_app_progress(
        &self,
        version: &str,
        downloaded_bytes: u64,
        total_bytes: u64,
    ) {
        if !self.enabled {
            return;
        }

        self.update_progress(
            progress_key("pv", "app", version),
            format!("Downloading PV {version}"),
            downloaded_bytes,
            total_bytes,
        );
    }

    fn update_resource_progress(
        &self,
        resource: &str,
        track: &str,
        artifact_version: &str,
        downloaded_bytes: u64,
        total_bytes: u64,
    ) {
        if !self.enabled {
            return;
        }

        let key = progress_key(resource, track, artifact_version);
        let label = progress_label(resource, track, artifact_version);
        self.update_progress(key, label, downloaded_bytes, total_bytes);
    }

    fn update_progress(&self, key: String, label: String, downloaded_bytes: u64, total_bytes: u64) {
        if !self.enabled {
            return;
        }

        let mut bars = self.bars.borrow_mut();
        {
            let bar = bars
                .entry(key.clone())
                .or_insert_with(|| progress_bar(total_bytes, label));
            bar.set_position(downloaded_bytes.min(total_bytes));
        }

        if downloaded_bytes >= total_bytes
            && let Some(bar) = bars.remove(&key)
        {
            bar.finish_and_clear();
        }
    }

    fn start_artifact(&self, artifact: &ManifestArtifact) {
        self.update_resource_progress(
            artifact.resource_name().as_str(),
            artifact.track().as_str(),
            artifact.artifact_version().as_str(),
            0,
            artifact.size(),
        );
    }

    fn advance_artifact(&self, artifact: &ManifestArtifact, downloaded_bytes: u64) {
        self.update_resource_progress(
            artifact.resource_name().as_str(),
            artifact.track().as_str(),
            artifact.artifact_version().as_str(),
            downloaded_bytes,
            artifact.size(),
        );
    }
}

impl DownloadProgress for DownloadProgressRenderer {
    fn report(&self, event: DownloadProgressEvent<'_>) {
        match event {
            DownloadProgressEvent::Started { artifact } => {
                self.start_artifact(artifact);
            }
            DownloadProgressEvent::Advanced {
                artifact,
                downloaded_bytes,
            }
            | DownloadProgressEvent::Finished {
                artifact,
                downloaded_bytes,
            } => {
                self.advance_artifact(artifact, downloaded_bytes);
            }
        }
    }
}

impl JobEventHandler for DownloadProgressRenderer {
    fn download_progress(&mut self, progress: JobDownloadProgress) {
        self.update_resource_progress(
            &progress.resource,
            &progress.track,
            &progress.artifact_version,
            progress.downloaded_bytes,
            progress.total_bytes,
        );
    }
}

impl Drop for DownloadProgressRenderer {
    fn drop(&mut self) {
        let bars = self.bars.get_mut();
        for bar in bars.values() {
            bar.finish_and_clear();
        }
        bars.clear();
    }
}

fn progress_bar(total_bytes: u64, label: String) -> ProgressBar {
    let bar = ProgressBar::new(total_bytes);
    bar.set_message(label);
    if let Ok(style) = ProgressStyle::with_template(
        "{msg} [{wide_bar}] {bytes}/{total_bytes} {bytes_per_sec} ETA {eta}",
    ) {
        bar.set_style(style.progress_chars("=> "));
    }

    bar
}

fn progress_key(resource: &str, track: &str, artifact_version: &str) -> String {
    format!("{resource}:{track}:{artifact_version}")
}

fn progress_label(resource: &str, track: &str, artifact_version: &str) -> String {
    format!(
        "Downloading {} track {track} ({artifact_version})",
        display_resource(resource)
    )
}

fn display_resource(resource: &str) -> String {
    match resource {
        "frankenphp" => "FrankenPHP".to_string(),
        "mailpit" => "Mailpit".to_string(),
        "mysql" => "MySQL".to_string(),
        "php" => "PHP".to_string(),
        "postgres" => "Postgres".to_string(),
        "redis" => "Redis".to_string(),
        "rustfs" => "RustFS".to_string(),
        other => other.to_string(),
    }
}
