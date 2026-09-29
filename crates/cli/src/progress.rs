use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::time::{Duration, Instant};

use daemon::{JobDownloadProgress, JobEventHandler};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use resources::{DownloadProgress, DownloadProgressEvent, ManifestArtifact};

use crate::output::Output;

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
    /// Live download bars when `stderr` is a terminal; nothing otherwise.
    pub(crate) fn new(stderr: &Output<'_>) -> Self {
        let enabled = stderr.surface().decorated();
        Self::with_progress(enabled, None, progress_target(enabled))
    }
}

impl<'output> DownloadProgressRenderer<'output> {
    /// Live progress when `stderr` is a terminal, or sparse phase lines
    /// written to a non-terminal `stderr`.
    pub(crate) fn with_output(stderr: &'output mut Output<'_>) -> Self {
        let enabled = stderr.surface().decorated();
        Self::with_progress(enabled, Some(stderr.writer()), progress_target(enabled))
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
            || format!("Downloading PV {version}"),
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
        let label = || progress_label(resource, track, artifact_version);
        self.update_progress(key, label, downloaded_bytes, total_bytes);
    }

    fn update_progress(
        &self,
        key: String,
        label: impl FnOnce() -> String,
        downloaded_bytes: u64,
        total_bytes: u64,
    ) {
        if !self.enabled {
            return;
        }

        let mut bars = self.bars.borrow_mut();
        {
            let bar = bars
                .entry(key.clone())
                .or_insert_with(|| self.progress.add(progress_bar(total_bytes, label())));
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
        if matches!(
            message,
            "Reconciliation still running" | "Managed Resource update still running"
        ) {
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

/// A spinner naming a step while it runs, for steps whose rows appear only
/// once they finish. It is hidden unless `stderr` is a terminal; the caller
/// clears it with `finish_and_clear`.
pub(crate) fn step_spinner(stderr: &Output<'_>, label: &str) -> ProgressBar {
    if !stderr.surface().decorated() {
        return ProgressBar::hidden();
    }
    let spinner = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr());
    spinner.set_style(status_style(false));
    spinner.set_message(label.to_string());
    spinner.enable_steady_tick(Duration::from_millis(100));

    spinner
}

/// Live progress draws on stderr, so redirected stdout only receives the
/// durable result.
fn progress_target(enabled: bool) -> MultiProgress {
    if enabled {
        MultiProgress::with_draw_target(ProgressDrawTarget::stderr())
    } else {
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
    }
}

/// The terminal design's spinner frames; the last one is the finished state.
const SPINNER_FRAMES: [&str; 5] = ["◐", "◓", "◑", "◒", "◇"];

fn status_style(show_elapsed: bool) -> ProgressStyle {
    let template = if show_elapsed {
        "{spinner:.cyan} {msg} {elapsed:.dim}"
    } else {
        "{spinner:.cyan} {msg}"
    };

    ProgressStyle::with_template(template)
        .unwrap_or_else(|_error| ProgressStyle::default_spinner())
        .tick_strings(&SPINNER_FRAMES)
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
    if let Ok(style) = ProgressStyle::with_template(
        "{spinner:.cyan} {msg} {bar:24.green/dim} {percent:>3}% {binary_bytes_per_sec:.dim}",
    ) {
        bar.set_style(style.progress_chars("█░").tick_strings(&SPINNER_FRAMES));
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
    use crate::output::{Output, Surface};

    #[test]
    fn non_terminal_progress_prints_sparse_transitions() -> anyhow::Result<()> {
        let mut output = Vec::new();
        {
            let mut stderr = Output::new(&mut output, Surface::plain());
            let mut progress = DownloadProgressRenderer::with_output(&mut stderr);
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
            let mut stderr = Output::new(&mut output, Surface::plain());
            let mut progress = DownloadProgressRenderer::with_output(&mut stderr);
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

    #[test]
    fn terminal_heartbeats_preserve_active_phase() {
        let target = MultiProgress::with_draw_target(ProgressDrawTarget::hidden());
        let mut progress = DownloadProgressRenderer::with_progress(true, None, target);
        assert!(progress.enabled);
        progress.progress("gateway");
        let active = progress
            .status
            .as_ref()
            .map(indicatif::ProgressBar::message);

        progress.log("Reconciliation still running");
        let after_reconciliation_heartbeat = progress
            .status
            .as_ref()
            .map(indicatif::ProgressBar::message);

        progress.log("Managed Resource update still running");
        let after_update_heartbeat = progress
            .status
            .as_ref()
            .map(indicatif::ProgressBar::message);

        progress.log("System reconciliation started");
        let after_normal_log = progress
            .status
            .as_ref()
            .map(indicatif::ProgressBar::message);

        insta::assert_debug_snapshot!(
            (active, after_reconciliation_heartbeat, after_update_heartbeat, after_normal_log),
            @r#"
        (
            Some(
                "Reconciliation phase: Gateway",
            ),
            Some(
                "Reconciliation phase: Gateway",
            ),
            Some(
                "Reconciliation phase: Gateway",
            ),
            Some(
                "System reconciliation started",
            ),
        )
        "#);
    }
}
