"""Runs fsevents-probe in parallel, optionally under load, and summarizes the results.

Usage: python3 run-probe.py <probe-binary> <out.jsonl> <processes> <rounds-each> <load: none|heavy>

Heavy load mixes CPU hashing, fsync writes, and a storm of file creates/renames/chmods/deletes,
because the CI failures happened while the full test suite was writing files everywhere.
"""

import json
import os
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

probe, out_path, processes, rounds, load = sys.argv[1], Path(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), sys.argv[5]
scratch = Path(tempfile.mkdtemp(prefix="pv511-probe-"))
cpus = os.cpu_count() or 4

CPU = "import hashlib\nb=b'x'*65536\nwhile True: hashlib.sha256(b).digest()"
FSYNC = "import os,sys\nf=open(sys.argv[1],'wb',buffering=0)\nb=b'x'*4096\nwhile True:\n f.seek(0);f.write(b);os.fsync(f.fileno())"
STORM = (
    "import os,sys\nd=sys.argv[1];os.makedirs(d,exist_ok=True);i=0\n"
    "while True:\n"
    " p=os.path.join(d,f'f{i%200}');t=p+'.tmp'\n"
    " open(t,'w').write('x'*64);os.chmod(t,0o600);os.rename(t,p);os.chmod(p,0o600)\n"
    " if i%3==0: os.unlink(p)\n"
    " i+=1"
)

workers = []
try:
    if load == "heavy":
        for _ in range(cpus * 2):
            workers.append(subprocess.Popen([sys.executable, "-c", CPU], start_new_session=True))
        for index in range(4):
            workers.append(subprocess.Popen([sys.executable, "-c", FSYNC, str(scratch / f"fsync-{index}")], start_new_session=True))
        for index in range(cpus):
            workers.append(subprocess.Popen([sys.executable, "-c", STORM, str(scratch / f"storm-{index}")], start_new_session=True))
        time.sleep(2)

    started = time.monotonic()
    probes = []
    for index in range(processes):
        directory = scratch / f"probe-{index}"
        directory.mkdir()
        probes.append(subprocess.Popen([probe, str(rounds), str(directory), f"{load}{index}"], stdout=subprocess.PIPE, text=True))
    lines = []
    for process in probes:
        output, _ = process.communicate()
        if process.returncode != 0:
            sys.exit(f"probe exited {process.returncode}")
        lines.extend(output.splitlines())
    elapsed = time.monotonic() - started
finally:
    for worker in workers:
        os.killpg(worker.pid, signal.SIGKILL)
    for worker in workers:
        worker.wait()
    shutil.rmtree(scratch, ignore_errors=True)

out_path.write_text("\n".join(lines) + "\n")
records = [json.loads(line) for line in lines]


def pct(values, q):
    values = sorted(values)
    return values[min(len(values) - 1, int(q * len(values)))] if values else float("nan")


print(f"load={load} rounds={len(records)} elapsed={elapsed:.0f}s")
print("\nLate setup events (setup's own pv.yml event delivered to a stream started after the save):")
buckets = [(0, 2), (2, 5), (5, 10), (10, 20), (20, 50), (50, 100), (100, 1e9)]
for low, high in buckets:
    group = [r for r in records if low <= r["setup_end_to_stream_start_ms"] < high]
    if group:
        late = [r for r in group if r["late_config_events"] > 0]
        print(f"  save->watch {low:>4}-{high if high < 1e9 else 'inf':>4} ms: {len(late):>5}/{len(group):<5} rounds had one")
late_all = [r for r in records if r["late_config_events"] > 0]
if late_all:
    own = sum(all(e["own"] for e in r["late_events"] if e["path"] == "/pv.yml") for r in late_all)
    print(f"  all marked as this process's own event: {own}/{len(late_all)}")
    print(f"  widest save->watch gap that still got one: {max(r['setup_end_to_stream_start_ms'] for r in late_all):.1f} ms")

lags = [r["action_first_lag_ms"] for r in records]
print("\nDelivery delay for the edit (save finished -> first event arrives):")
print(f"  p50={pct(lags, .5):.1f} p90={pct(lags, .9):.1f} p99={pct(lags, .99):.1f} max={max(lags):.1f} ms")
split = [r for r in records if r["action_config_callbacks"] > 1]
spreads = [r["action_config_spread_ms"] for r in records]
print("\nOne edit's pv.yml events split over several deliveries:")
print(f"  {len(split)}/{len(records)} rounds; spread p99={pct(spreads, .99):.1f} max={max(spreads):.1f} ms; >50 ms: {sum(s > 50 for s in spreads)}")
