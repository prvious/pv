"""Summarizes issue #511 trace logs written by stress-tests.py.

Usage: python3 analyze.py <out-dir> [--timelines]
"""

import json
import re
import sys
from pathlib import Path

out = Path(sys.argv[1])
show_timelines = "--timelines" in sys.argv
LINE = re.compile(r"PV511 Instant \{ tv_sec: (\d+), tv_nsec: (\d+) \} (\S+) ?(.*)")
TMP = re.compile(r"/private/var/folders/\S*?/T/\.tmp\w+")


def parse(path):
    events, other = [], []
    for raw in path.read_text(errors="replace").splitlines():
        match = LINE.search(raw)
        if match:
            seconds, nanos, tag, rest = match.groups()
            events.append((int(seconds) + int(nanos) / 1e9, tag, TMP.sub("", rest)))
        elif re.search(r"Error|panicked|assertion|left:|right:|FAILED", raw):
            # libtest prints a returned error on the test's own line: "test name ... Error: ...".
            other.append(re.sub(r"^test \S+ \.\.\. ", "", raw.strip()))
    return events, other


def pct(values, q):
    values = sorted(values)
    return values[min(len(values) - 1, int(q * len(values)))] if values else float("nan")


def first(events, tag, after=0.0):
    return next((t for t, g, _ in events if g == tag and t >= after), None)


def segments(events):
    """One segment per watcher setup: a table row, or the whole test."""
    starts = [i for i, (_, tag, _) in enumerate(events) if tag in ("TEST_ROW",)]
    if not starts:
        return [("linked", events)]
    bounds = starts + [len(events)]
    return [(events[a][2], events[a:b]) for a, b in zip(bounds, bounds[1:])]


results = json.loads((out / "results.json").read_text())
watcher, jobs, failures = [], [], []
for result in results:
    events, errors = parse(out / result["log"])
    if result["exit"] != 0:
        failures.append((result, events, errors))
    if "queued_foreground" in result["test"]:
        open_begin = first(events, "ADMIT_OPEN_BEGIN")
        times = {tag: first(events, tag) for tag in ("ADMIT_OPEN_END", "ADMIT_RECORDED", "ADMIT_LOGGED", "ADMIT_CLOSED", "JOB_ACCEPTED_WAIT", "JOB_ACCEPTED")}
        if open_begin and times["ADMIT_CLOSED"]:
            jobs.append({"admit_ms": (times["ADMIT_CLOSED"] - open_begin) * 1000, "failed": result["exit"] != 0, "errors": errors})
        else:
            jobs.append({"admit_ms": None, "failed": result["exit"] != 0, "errors": errors})
        continue
    for name, segment in segments(events):
        setup = first(segment, "TEST_SETUP_DONE")
        started = first(segment, "FSE_STREAM_STARTED", setup or 0)
        change = first(segment, "TEST_CHANGE_BEGIN")
        if setup is None or started is None:
            continue
        end = change if change else segment[-1][0]
        late = [rest for t, tag, rest in segment if tag == "FSE_RAW" and started <= t < end]
        record = {"test": result["test"], "row": name, "log": result["log"], "gap_ms": (started - setup) * 1000, "late": late}
        if change:
            # Stop at the test's last look, so deleting the temporary directory afterwards doesn't count.
            stop = first(segment, "TEST_REQUESTED", change) or max((t for t, tag, _ in segment if tag == "DEBOUNCE_FLUSH"), default=segment[-1][0])
            requests = [t for t, tag, _ in segment if tag == "DEBOUNCE_REQUEST" and change <= t <= stop]
            callbacks = [t for t, tag, _ in segment if tag == "FSE_CALLBACK" and change <= t <= stop]
            record.update({
                "flushes": sum(1 for t, tag, _ in segment if tag == "DEBOUNCE_FLUSH" and t >= change),
                "request_span_ms": (max(requests) - min(requests)) * 1000 if requests else None,
                "callbacks": len(callbacks),
                "callback_span_ms": (max(callbacks) - min(callbacks)) * 1000 if callbacks else None,
                "first_request_ms": (min(requests) - change) * 1000 if requests else None,
            })
        watcher.append(record)

print(f"{len(results)} runs, {len(failures)} failed")
for test in sorted({result["test"] for result in results}):
    runs = [result for result in results if result["test"] == test]
    reasons = {}
    for result, _events, errors in failures:
        if result["test"] == test:
            reason = re.sub(r"[a-z0-9]{10}", "<id>", next((e for e in errors if e.startswith(("Error", "left"))), "?"))
            reasons[reason] = reasons.get(reason, 0) + 1
    print(f"  {test}: {sum(r['exit'] != 0 for r in runs)}/{len(runs)} failed {reasons}")
for test in sorted({record["test"] for record in watcher}):
    group = [record for record in watcher if record["test"] == test]
    print(f"\n{test.rsplit('::', 1)[1]}: {len(group)} watcher setups")
    gaps = [record["gap_ms"] for record in group]
    print(f"  setup save -> stream started: p50={pct(gaps, .5):.1f} p90={pct(gaps, .9):.1f} min={min(gaps):.1f} max={max(gaps):.1f} ms")
    late = [record for record in group if record["late"]]
    print(f"  setups with raw events after the stream started and before the change: {len(late)}")
    for record in late[:5]:
        print(f"    {record['log']} {record['row']} gap={record['gap_ms']:.1f}ms {record['late']}")
    changed = [record for record in group if "flushes" in record]
    if changed:
        spans = [record["request_span_ms"] for record in changed if record["request_span_ms"] is not None]
        cb_spans = [record["callback_span_ms"] for record in changed if record["callback_span_ms"] is not None]
        print(f"  after the change: flushes {sorted({record['flushes'] for record in changed})}, >1 flush: {sum(record['flushes'] > 1 for record in changed)}")
        # A run can time out before any request or delivery; pct reports an empty sample as nan.
        print(f"  first..last request span ({len(spans)} samples): p50={pct(spans, .5):.1f} p99={pct(spans, .99):.1f} max={pct(spans, 1):.1f} ms")
        print(f"  deliveries per change: {sorted({record['callbacks'] for record in changed})}; delivery span ({len(cb_spans)} samples) p99={pct(cb_spans, .99):.1f} max={pct(cb_spans, 1):.1f} ms")

admits = [job["admit_ms"] for job in jobs if job["admit_ms"] is not None]
if admits:
    print(f"\njob test: {len(jobs)} runs, {sum(job['failed'] for job in jobs)} failed; admission p50={pct(admits, .5):.1f} p99={pct(admits, .99):.1f} max={pct(admits, 1):.1f} ms")

for result, events, errors in failures:
    print(f"\nFAILED {result['log']}: {errors[:6]}")
    if show_timelines and events:
        origin = events[0][0]
        for t, tag, rest in events:
            if tag not in ("CHANGES_CHECK",):
                print(f"  {1000 * (t - origin):9.2f} {tag} {rest[:150]}")
