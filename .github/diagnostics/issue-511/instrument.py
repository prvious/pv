"""Adds issue #511 trace lines to a PV checkout and to a copy of notify 8.2.0.

Usage: python3 instrument.py <pv-checkout> <notify-copy>

Every trace line starts with `PV511 <Instant>` so lines from notify's FSEvents thread and from PV
can be put on one clock. Nothing here changes behavior: it only prints to stderr, and adds
FSEvents' MarkSelf flag, which only labels events this process caused.
"""

import sys
from pathlib import Path

pv, notify = Path(sys.argv[1]), Path(sys.argv[2])
T = 'std::time::Instant::now()'


def edit(path, *changes):
    text = path.read_text()
    for old, new in changes:
        if text.count(old) != 1:
            sys.exit(f"{path}: expected one match for {old!r}, found {text.count(old)}")
        text = text.replace(old, new)
    path.write_text(text)


edit(
    notify / "src/fsevent.rs",
    (
        "flags: fs::kFSEventStreamCreateFlagFileEvents | fs::kFSEventStreamCreateFlagNoDefer,",
        "flags: fs::kFSEventStreamCreateFlagFileEvents | fs::kFSEventStreamCreateFlagNoDefer\n"
        "                | fs::kFSEventStreamCreateFlagMarkSelf,",
    ),
    (
        "                    fs::FSEventStreamStart(stream);\n",
        f'                    eprintln!("PV511 {{:?}} FSE_STREAM_START_BEGIN current_id={{}}", {T}, fs::FSEventsGetCurrentEventId());\n'
        "                    fs::FSEventStreamStart(stream);\n"
        f'                    eprintln!("PV511 {{:?}} FSE_STREAM_STARTED current_id={{}}", {T}, fs::FSEventsGetCurrentEventId());\n',
    ),
    (
        "    _event_ids: *const fs::FSEventStreamEventId,     // const FSEventStreamEventId eventIds[]\n) {\n",
        "    event_ids: *const fs::FSEventStreamEventId,     // const FSEventStreamEventId eventIds[]\n) {\n"
        f'    eprintln!("PV511 {{:?}} FSE_CALLBACK events={{}}", {T}, num_events);\n',
    ),
    (
        "        if !handle_event {\n            continue;\n        }\n",
        f'        eprintln!("PV511 {{:?}} FSE_RAW id={{}} flags={{:#x}} own={{}} handled={{}} path={{}}", {T}, *event_ids.add(p), flag.bits(), flag.contains(StreamFlags::OWN_EVENT), handle_event, path.display());\n'
        "        if !handle_event {\n            continue;\n        }\n",
    ),
)

edit(
    pv / "crates/daemon/src/watcher.rs",
    (
        "        while let Some(message) = messages.recv().await {\n            let watcher = self.clone();\n",
        "        while let Some(message) = messages.recv().await {\n"
        "            let description = match &message {\n"
        '                WatchMessage::Event(Ok(event)) => format!("{:?} {:?}", event.kind, event.paths),\n'
        '                WatchMessage::Event(Err(error)) => format!("error {error}"),\n'
        '                WatchMessage::Restarted => "Restarted".to_string(),\n'
        "            };\n"
        f'            eprintln!("PV511 {{:?}} WATCH_MESSAGE {{description}}", {T});\n'
        "            let watcher = self.clone();\n",
    ),
    (
        "            for scope in scopes {\n                debouncer.request(scope).await;\n",
        f'            eprintln!("PV511 {{:?}} WATCH_SCOPES {{scopes:?}}", {T});\n'
        "            for scope in scopes {\n                debouncer.request(scope).await;\n",
    ),
    (
        "        // The next reload tries these again.\n",
        f'        eprintln!("PV511 {{:?}} WATCH_COMMITTED {{:?}}", {T}, watched.keys().collect::<Vec<_>>());\n'
        "        // The next reload tries these again.\n",
    ),
    (
        "            let config = config_modified_times(&project.path);\n            if config != project.config {\n                project.config = config;\n                scopes.extend(ReconciliationScope::project(project_id.clone()).ok());\n            }\n            if let Some((git, entries)) = &mut project.worktrees\n                && worktree_entries_changed(git, entries)\n",
        "            let config = config_modified_times(&project.path);\n"
        f'            eprintln!("PV511 {{:?}} CHANGES_CHECK {{project_id}} saved={{:?}} now={{config:?}}", {T}, project.config);\n'
        "            if config != project.config {\n                project.config = config;\n                scopes.extend(ReconciliationScope::project(project_id.clone()).ok());\n            }\n            if let Some((git, entries)) = &mut project.worktrees\n                && worktree_entries_changed(git, entries)\n",
    ),
    (
        "                if project.config_paths.contains(path) {\n",
        "                if project.config_paths.contains(path) {\n"
        f'                    eprintln!("PV511 {{:?}} EVENT_CONFIG_PATH {{project_id}} {{path}} {{:?}}", {T}, event.kind);\n',
    ),
    (
        "                    let config = config_modified_times(&project.path);\n                    if config != project.config {\n",
        "                    let config = config_modified_times(&project.path);\n"
        f'                    eprintln!("PV511 {{:?}} EVENT_DIRECTORY {{project_id}} {{path}} saved={{:?}} now={{config:?}}", {T}, project.config);\n'
        "                    if config != project.config {\n",
    ),
    # Tests.
    (
        "        async fn run(&self) -> Result<RunningWatcher> {\n            let (watcher, messages) = self.watch()?;\n",
        "        async fn run(&self) -> Result<RunningWatcher> {\n"
        f'            eprintln!("PV511 {{:?}} TEST_RUN_BEGIN", {T});\n'
        "            let (watcher, messages) = self.watch()?;\n"
        f'            eprintln!("PV511 {{:?}} TEST_WATCH_STARTED", {T});\n',
    ),
    (
        "            let startup = running.settle().await;\n",
        "            let startup = running.settle().await;\n"
        f'            eprintln!("PV511 {{:?}} TEST_SETTLED startup={{startup:?}}", {T});\n',
    ),
    (
        "        for (name, existing_config, change) in rows {\n            let fixture = Fixture::new()?;\n",
        "        for (name, existing_config, change) in rows {\n"
        f'            eprintln!("PV511 {{:?}} TEST_ROW {{name}}", {T});\n'
        "            let fixture = Fixture::new()?;\n",
    ),
    (
        "            let mut watcher = fixture.run().await?;\n\n            change(&project_path)?;\n",
        f'            eprintln!("PV511 {{:?}} TEST_SETUP_DONE", {T});\n'
        "            let mut watcher = fixture.run().await?;\n\n"
        f'            eprintln!("PV511 {{:?}} TEST_CHANGE_BEGIN", {T});\n'
        "            change(&project_path)?;\n"
        f'            eprintln!("PV511 {{:?}} TEST_CHANGE_END", {T});\n',
    ),
    (
        "        state::fs::symlink_file(&target, &project_path.join(\"pv.yml\"))?;\n        let mut watcher = fixture.run().await?;\n\n        write(&target, \"php: '8.4'\\n\")?;\n\n        assert_eq!(watcher.requested().await?, vec![scope]);\n",
        "        state::fs::symlink_file(&target, &project_path.join(\"pv.yml\"))?;\n"
        f'        eprintln!("PV511 {{:?}} TEST_SETUP_DONE", {T});\n'
        "        let mut watcher = fixture.run().await?;\n\n"
        f'        eprintln!("PV511 {{:?}} TEST_CHANGE_BEGIN", {T});\n'
        "        write(&target, \"php: '8.4'\\n\")?;\n"
        f'        eprintln!("PV511 {{:?}} TEST_CHANGE_END", {T});\n\n'
        "        let requested = watcher.requested().await?;\n"
        f'        eprintln!("PV511 {{:?}} TEST_REQUESTED {{requested:?}}", {T});\n'
        "        assert_eq!(requested, vec![scope]);\n",
    ),
)

edit(
    pv / "crates/daemon/src/reconciliation.rs",
    (
        "    pub async fn request(&self, scope: ReconciliationScope) {\n        let should_spawn = {\n",
        "    pub async fn request(&self, scope: ReconciliationScope) {\n"
        f'        eprintln!("PV511 {{:?}} DEBOUNCE_REQUEST {{scope}}", {T});\n'
        "        let should_spawn = {\n",
    ),
    (
        "            std::mem::take(&mut inner.pending)\n        };\n",
        "            std::mem::take(&mut inner.pending)\n        };\n"
        f'        eprintln!("PV511 {{:?}} DEBOUNCE_FLUSH {{scopes:?}}", {T});\n',
    ),
)

edit(
    pv / "crates/daemon/src/jobs.rs",
    (
        "fn start_reconciliation_job(paths: &PvPaths, scope: &str) -> Result<String, DaemonError> {\n"
        "    let mut database = Database::open(paths)?;\n"
        "    let job = database.start_job(\"reconcile\", scope)?;\n"
        "    structured_log::job_started(paths, &job.id, \"reconcile\", scope);\n\n"
        "    Ok(job.id)\n}\n",
        "fn start_reconciliation_job(paths: &PvPaths, scope: &str) -> Result<String, DaemonError> {\n"
        f'    eprintln!("PV511 {{:?}} ADMIT_OPEN_BEGIN", {T});\n'
        "    let mut database = Database::open(paths)?;\n"
        f'    eprintln!("PV511 {{:?}} ADMIT_OPEN_END", {T});\n'
        "    let job = database.start_job(\"reconcile\", scope)?;\n"
        f'    eprintln!("PV511 {{:?}} ADMIT_RECORDED", {T});\n'
        "    structured_log::job_started(paths, &job.id, \"reconcile\", scope);\n"
        f'    eprintln!("PV511 {{:?}} ADMIT_LOGGED", {T});\n'
        "    drop(database);\n"
        f'    eprintln!("PV511 {{:?}} ADMIT_CLOSED", {T});\n\n'
        "    Ok(job.id)\n}\n",
    ),
    (
        "        let mut reader = protocol::transport(client);\n"
        "        let accepted = real::timeout(Duration::from_millis(100), reader.next())\n"
        "            .await?\n",
        "        let mut reader = protocol::transport(client);\n"
        f'        eprintln!("PV511 {{:?}} JOB_ACCEPTED_WAIT", {T});\n'
        "        let accepted = real::timeout(Duration::from_millis(100), reader.next())\n"
        '            .await\n            .map_err(|error| anyhow::anyhow!("accepted wait: {error}"))?\n',
    ),
    (
        "        assert_eq!(accepted[\"status\"], \"accepted\");\n"
        "        assert!(\n"
        "            real::timeout(Duration::from_millis(20), reader.next())\n"
        "                .await\n"
        "                .is_err()\n"
        "        );\n\n"
        "        running.finish();\n",
        "        assert_eq!(accepted[\"status\"], \"accepted\");\n"
        f'        eprintln!("PV511 {{:?}} JOB_ACCEPTED", {T});\n'
        "        assert!(\n"
        "            real::timeout(Duration::from_millis(20), reader.next())\n"
        "                .await\n"
        "                .is_err(),\n"
        '            "quiet check: an event arrived before the first job finished"\n'
        "        );\n\n"
        f'        eprintln!("PV511 {{:?}} JOB_RELEASE", {T});\n'
        "        running.finish();\n",
    ),
    (
        "            let line = real::timeout(Duration::from_millis(500), reader.next())\n"
        "                .await?\n"
        "                .ok_or_else(|| anyhow::anyhow!(\"job stream ended before completion\"))??;\n"
        "            let event = serde_json::from_str::<serde_json::Value>(&line)?;\n"
        "            let completed = event[\"type\"] == \"job_completed\";\n",
        "            let line = real::timeout(Duration::from_millis(500), reader.next())\n"
        "                .await\n"
        '                .map_err(|error| anyhow::anyhow!("event wait after {} events: {error}", events.len()))?\n'
        "                .ok_or_else(|| anyhow::anyhow!(\"job stream ended before completion\"))??;\n"
        "            let event = serde_json::from_str::<serde_json::Value>(&line)?;\n"
        f'            eprintln!("PV511 {{:?}} JOB_EVENT {{}}", {T}, event["type"]);\n'
        "            let completed = event[\"type\"] == \"job_completed\";\n",
    ),
)
print("instrumented")

# Collect trace lines in memory and print them when the test ends, so tracing doesn't slow the
# watcher down while it works. Printing from FSEvents' thread and every step under load made the
# duplicate failure 30x more likely than in the plain build.
import re

STATEMENT = re.compile(r'eprintln!\("PV511 \{:\?\} (.*?)", std::time::Instant::now\(\)(.*)\);[ \t]*$', re.M)


def buffered(path, owner):
    text = path.read_text()
    text, count = STATEMENT.subn(
        lambda match: f'{owner}::PV511_LOG.lock().map(|mut log| log.push((std::time::Instant::now(), format!("{match.group(1)}"{match.group(2)})))).ok();',
        text,
    )
    path.write_text(text)
    return count


counts = [
    buffered(notify / "src/fsevent.rs", "crate"),
    buffered(pv / "crates/daemon/src/watcher.rs", "notify"),
    buffered(pv / "crates/daemon/src/reconciliation.rs", "notify"),
    buffered(pv / "crates/daemon/src/jobs.rs", "notify"),
]
print("buffered trace statements:", counts)

edit(
    notify / "src/lib.rs",
    (
        "pub use config::{Config, RecursiveMode};\n",
        "pub use config::{Config, RecursiveMode};\n\n"
        "/// Issue #511 trace lines, printed by PV's tests when they end.\n"
        "#[doc(hidden)]\n"
        "pub static PV511_LOG: std::sync::Mutex<Vec<(std::time::Instant, String)>> = std::sync::Mutex::new(Vec::new());\n",
    ),
)
DUMP = (
    "    fn pv511_dump() {\n"
    "        if let Ok(mut log) = notify::PV511_LOG.lock() {\n"
    "            for (time, line) in log.drain(..) {\n"
    '                eprintln!("PV511 {time:?} {line}");\n'
    "            }\n"
    "        }\n"
    "    }\n\n"
)
edit(
    pv / "crates/daemon/src/watcher.rs",
    ("    struct RunningWatcher {\n", DUMP + "    struct RunningWatcher {\n"),
    ("    impl Drop for RunningWatcher {\n        fn drop(&mut self) {\n", "    impl Drop for RunningWatcher {\n        fn drop(&mut self) {\n            pv511_dump();\n"),
)
edit(
    pv / "crates/daemon/src/jobs.rs",
    (
        "    async fn queued_foreground_reconciliation_streams_only_after_its_turn() -> anyhow::Result<()> {\n",
        "    async fn queued_foreground_reconciliation_streams_only_after_its_turn() -> anyhow::Result<()> {\n"
        "        struct Pv511Dump;\n"
        "        impl Drop for Pv511Dump {\n"
        "            fn drop(&mut self) {\n"
        "                if let Ok(mut log) = notify::PV511_LOG.lock() {\n"
        "                    for (time, line) in log.drain(..) {\n"
        '                        eprintln!("PV511 {time:?} {line}");\n'
        "                    }\n"
        "                }\n"
        "            }\n"
        "        }\n"
        "        let _pv511_dump = Pv511Dump;\n",
    ),
)
print("buffered")
