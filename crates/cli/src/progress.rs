use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::time::{Duration, Instant};

use daemon::{JobDownloadProgress, JobEventHandler};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use resources::{DownloadProgress, DownloadProgressEvent, ManifestArtifact};

const WAITING_MESSAGE: &str = "Waiting for the reconciliation slot";

pub(crate) struct DownloadProgressRenderer<'output> {
    enabled: bool,
    progress: MultiProgress,
    status: Option<ProgressBar>,
    bars: RefCell<BTreeMap<String, ProgressBar>>,
    output: Option<&'output mut dyn Write>,
    waiting_since: Option<Instant>,
}

impl DownloadProgressRenderer<'static> {
    pub(crate) fn new(enabled: bool) -> Self {
        Self::with_progress(enabled, None, progress_target(enabled))
    }
}

impl<'output> DownloadProgressRenderer<'output> {
    pub(crate) fn with_output(enabled: bool, output: &'output mut dyn Write) -> Self {
        Self::with_progress(enabled, Some(output), progress_target(enabled))
    }

    fn with_progress(
        enabled: bool,
        output: Option<&'output mut dyn Write>,
        progress: MultiProgress,
    ) -> Self {
        Self {
            enabled,
            progress,
            status: None,
            bars: RefCell::new(BTreeMap::new()),
            output,
            waiting_since: None,
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
                .or_insert_with(|| self.progress.add(progress_bar(total_bytes, label)));
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

    fn transition(&mut self, message: &str, show_elapsed: bool) {
        if self.enabled {
            let status = self.status.get_or_insert_with(|| {
                let status = self.progress.insert(0, ProgressBar::new_spinner());
                status.enable_steady_tick(Duration::from_millis(100));

                status
            });
            status.set_style(status_style(show_elapsed));
            status.set_message(message.to_string());
            status.tick();

            return;
        }

        if let Some(output) = self.output.as_deref_mut() {
            let _write_result = writeln!(output, "{message}");
        }
    }
}

impl DownloadProgress for DownloadProgressRenderer<'_> {
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

impl JobEventHandler for DownloadProgressRenderer<'_> {
    fn job_accepted(&mut self, _job_id: &str) {
        self.waiting_since = Some(Instant::now());
        if self.enabled {
            self.transition(WAITING_MESSAGE, true);
        } else {
            self.transition(&format!("{WAITING_MESSAGE} (elapsed: 0s)"), false);
        }
    }

    fn job_started(&mut self, kind: &str, _scope: &str) {
        let wait = self
            .waiting_since
            .take()
            .map_or(Duration::ZERO, |started_at| started_at.elapsed());
        let work = if kind == "update" {
            "Managed Resource update"
        } else {
            "Reconciliation"
        };
        self.transition(
            &format!("{work} slot acquired after {}", elapsed_label(wait)),
            false,
        );
    }

    fn progress(&mut self, message: &str) {
        if let Some(message) = progress_message(message) {
            self.transition(&message, false);
        }
    }

    fn log(&mut self, message: &str) {
        if message == WAITING_MESSAGE {
            if self.enabled {
                self.transition(message, true);
            }
            return;
        }
        if !self.enabled
            && matches!(
                message,
                "Reconciliation still running" | "Managed Resource update still running"
            )
        {
            return;
        }
        self.transition(message, false);
    }

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

impl Drop for DownloadProgressRenderer<'_> {
    fn drop(&mut self) {
        if let Some(status) = self.status.take() {
            status.finish_and_clear();
        }
        let bars = self.bars.get_mut();
        for bar in bars.values() {
            bar.finish_and_clear();
        }
        bars.clear();
    }
}

fn progress_target(enabled: bool) -> MultiProgress {
    if enabled {
        MultiProgress::with_draw_target(ProgressDrawTarget::stdout())
    } else {
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
    }
}

fn status_style(show_elapsed: bool) -> ProgressStyle {
    let template = if show_elapsed {
        "{spinner} {msg} [{elapsed_precise}]"
    } else {
        "{spinner} {msg}"
    };

    ProgressStyle::with_template(template).unwrap_or_else(|_error| ProgressStyle::default_spinner())
}

fn elapsed_label(elapsed: Duration) -> String {
    if elapsed < Duration::from_secs(1) {
        return "<1s".to_string();
    }

    format!("{}s", elapsed.as_secs())
}

fn progress_message(message: &str) -> Option<String> {
    let phase = match message {
        "demand_discovery" => "Demand discovery",
        "manifest" => "Artifact manifest",
        "download" => "Artifact download",
        "install" => "Artifact installation",
        "project_apply" => "Project configuration",
        "resources" => "Managed Resources",
        "workers" => "PHP workers",
        "gateway" => "Gateway",
        "finalization" => "Finalization",
        _ => return None,
    };

    Some(format!("Reconciliation phase: {phase}"))
}

fn progress_bar(total_bytes: u64, label: String) -> ProgressBar {
    let bar = ProgressBar::new(total_bytes);
    bar.set_message(label);
    if let Ok(style) = ProgressStyle::with_template("{msg} [{wide_bar}] {bytes}/{total_bytes}") {
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
        "caddy" => "Caddy".to_string(),
        "composer" => "Composer".to_string(),
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

#[cfg(test)]
mod tests {
    use daemon::{JobDownloadProgress, JobEventHandler};
    use indicatif::{MultiProgress, ProgressDrawTarget};
    use insta::assert_snapshot;

    use super::DownloadProgressRenderer;

    #[test]
    fn non_terminal_progress_prints_sparse_transitions() -> anyhow::Result<()> {
        let mut output = Vec::new();
        {
            let mut progress = DownloadProgressRenderer::with_output(false, &mut output);
            progress.job_accepted("job_1");
            progress.log("Waiting for the reconciliation slot");
            progress.log("Waiting for the reconciliation slot");
            progress.job_started("reconcile", "system");
            progress.log("System reconciliation started");
            progress.progress("demand_discovery");
            progress.progress("resources");
            progress.log("Reconciliation still running");
            progress.log("Reconciliation still running");
            progress.progress("finalization");
        }
        let output = String::from_utf8(output)?;

        assert_snapshot!(output, @r"
        Waiting for the reconciliation slot (elapsed: 0s)
        Reconciliation slot acquired after <1s
        System reconciliation started
        Reconciliation phase: Demand discovery
        Reconciliation phase: Managed Resources
        Reconciliation phase: Finalization
        ");

        Ok(())
    }

    #[test]
    fn non_terminal_update_prints_every_known_phase() -> anyhow::Result<()> {
        let mut output = Vec::new();
        {
            let mut progress = DownloadProgressRenderer::with_output(false, &mut output);
            progress.job_accepted("job_1");
            progress.job_started("update", "system");
            for phase in [
                "demand_discovery",
                "manifest",
                "download",
                "install",
                "project_apply",
                "resources",
                "workers",
                "gateway",
                "finalization",
                "unknown",
            ] {
                progress.progress(phase);
            }
        }
        let output = String::from_utf8(output)?;

        assert_snapshot!(output, @r"
        Waiting for the reconciliation slot (elapsed: 0s)
        Managed Resource update slot acquired after <1s
        Reconciliation phase: Demand discovery
        Reconciliation phase: Artifact manifest
        Reconciliation phase: Artifact download
        Reconciliation phase: Artifact installation
        Reconciliation phase: Project configuration
        Reconciliation phase: Managed Resources
        Reconciliation phase: PHP workers
        Reconciliation phase: Gateway
        Reconciliation phase: Finalization
        ");

        Ok(())
    }

    #[test]
    fn terminal_progress_keeps_one_status_with_download_bars() {
        let target = MultiProgress::with_draw_target(ProgressDrawTarget::hidden());
        let mut progress = DownloadProgressRenderer::with_progress(true, None, target);
        progress.job_accepted("job_1");
        let waiting = progress
            .status
            .as_ref()
            .map(indicatif::ProgressBar::message);
        progress.job_started("reconcile", "system");
        progress.progress("download");
        progress.download_progress(JobDownloadProgress {
            resource: "redis".to_string(),
            track: "8.8".to_string(),
            artifact_version: "8.8.1-pv1".to_string(),
            downloaded_bytes: 42,
            total_bytes: 100,
        });
        let active = progress
            .status
            .as_ref()
            .map(indicatif::ProgressBar::message);
        let downloads = progress
            .bars
            .borrow()
            .values()
            .map(|bar| (bar.message(), bar.position(), bar.length()))
            .collect::<Vec<_>>();

        insta::assert_debug_snapshot!((waiting, active, downloads), @r#"
        (
            Some(
                "Waiting for the reconciliation slot",
            ),
            Some(
                "Reconciliation phase: Artifact download",
            ),
            [
                (
                    "Downloading Redis track 8.8 (8.8.1-pv1)",
                    42,
                    Some(
                        100,
                    ),
                ),
            ],
        )
        "#);
    }
}
