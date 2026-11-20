use std::time::Duration;

use anyhow::{Result, anyhow};
use camino_tempfile::tempdir;
use notify::{RecursiveMode, Watcher};
use platform::clock::real;
use tokio::sync::mpsc::unbounded_channel;

const MARKER_TIMEOUT: Duration = Duration::from_secs(10);
const MARKER_RETRY: Duration = Duration::from_secs(1);

/// Returns once FSEvents has numbered every file change made so far, so a daemon started
/// afterwards never receives the test's setup writes as new events and requests a reconciliation
/// the test didn't ask for.
///
/// A new stream receives the events numbered after it starts, and on a busy machine FSEvents
/// numbers a write some milliseconds after it happens. It numbers writes in order across the
/// machine, so once a marker written now reaches a separate stream, every earlier write has a lower
/// number. The watcher's own test fixture waits the same way.
pub(crate) async fn wait_for_file_events_before_now() -> Result<()> {
    let tempdir = tempdir()?;
    // Events name canonical paths, and macOS temporary directories sit behind a symlink.
    let directory = state::fs::canonicalize(tempdir.path())?;
    let (sender, mut events) = unbounded_channel();
    let mut marker_watcher = notify::recommended_watcher(move |event| {
        let _result = sender.send(event);
    })?;
    marker_watcher.watch(directory.as_std_path(), RecursiveMode::NonRecursive)?;
    let marker = directory.join("file-events-marker");
    state::fs::write_sensitive_file(&marker, "")?;

    real::timeout(MARKER_TIMEOUT, async {
        loop {
            match real::timeout(MARKER_RETRY, events.recv()).await {
                Ok(Some(event)) => {
                    let event = event?;
                    if event.paths.iter().any(|path| path == marker.as_std_path()) {
                        return Ok(());
                    }
                    // Under load FSEvents can fold events into one rescan of the directory, which
                    // may replace the marker's own event.
                    if event.need_rescan() {
                        state::fs::write_sensitive_file(&marker, "")?;
                    }
                }
                Ok(None) => return Err(anyhow!("marker watcher stopped")),
                // A dropped-events notice can name `/`, which never reaches this watcher, so the
                // marker is written again until one of its events arrives.
                Err(_elapsed) => state::fs::write_sensitive_file(&marker, "")?,
            }
        }
    })
    .await?
}
