# Issue #511 diagnostics

Not for merging. `.github/workflows/issue-511-diagnostics.yml` runs these scripts on a macOS 26
runner to find out why three daemon tests flake there:

- `watcher_enqueues_a_config_that_appears_conflicts_or_is_edited_in_place` (startup requested)
- `watcher_enqueues_an_edit_to_the_file_a_config_links_to` (one edit requested twice)
- `queued_foreground_reconciliation_streams_only_after_its_turn` (deadline has elapsed)

| File | What it does |
| --- | --- |
| `fsevents-probe.swift` | Without PV, saves a file the way PV does, starts an FSEvents stream the way notify 8.2.0 does, and records every raw event: whether the setup's own save still arrives after the stream starts, and how one save's events spread over deliveries. |
| `run-probe.py` | Runs the probe in parallel and summarizes it. |
| `stress-tests.py` | Runs each test in its own process, over and over, and keeps every log and exit code. |
| `load.py` | Runs the full workspace suite twice at once, the condition the CI failures happened in. |
| `instrument.py` | Adds trace lines to the watcher, the debouncer, the job admission, the tests, and a copy of notify (raw FSEvents ids, flags and paths, plus FSEvents' MarkSelf flag, which only labels this process's own events). Lines are kept in memory and printed when a test ends, so tracing doesn't slow the watcher down. |
| `analyze.py` | Summarizes the logs and prints every failed run's timeline. |

The job builds the unmodified tests first and runs them under load, so their failure rate isn't
affected by tracing, then rebuilds with tracing and runs them again for timelines. Everything is
uploaded as the `issue-511-macos-26` artifact; the job summary has the headline numbers.
