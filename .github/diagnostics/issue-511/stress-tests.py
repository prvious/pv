"""Runs single PV tests many times, each in its own process, and keeps every log.

Usage: python3 stress-tests.py <test-list.json> <out-dir> <seconds> <test>=<parallel> ...

Each test keeps <parallel> copies running until <seconds> pass. Logs go to
<out-dir>/<short-name>-<n>.log; <out-dir>/results.json lists every run's exit code, so a failure
can't hide behind a summary.
"""

import json
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

listing, out, seconds = sys.argv[1], Path(sys.argv[2]), float(sys.argv[3])
suite = json.load(open(listing))["rust-suites"]["daemon"]
binary, cwd = suite["binary-path"], suite["cwd"]
out.mkdir(parents=True, exist_ok=True)
deadline = time.monotonic() + seconds
results = []


def lane(test, short, lane_index):
    run = 0
    while time.monotonic() < deadline:
        log = out / f"{short}-{lane_index}-{run}.log"
        with open(log, "w") as output:
            code = subprocess.run(
                [binary, "--exact", test, "--nocapture", "--test-threads", "1"],
                cwd=cwd, stdout=output, stderr=subprocess.STDOUT,
            ).returncode
        results.append({"test": test, "log": log.name, "exit": code})
        run += 1


with ThreadPoolExecutor() as pool:
    futures = []
    for argument in sys.argv[4:]:
        test, parallel = argument.rsplit("=", 1)
        short = test.rsplit("::", 1)[1][:40]
        assert test in suite["testcases"], test
        for lane_index in range(int(parallel)):
            futures.append(pool.submit(lane, test, short, lane_index))
    for future in futures:
        future.result()

(out / "results.json").write_text(json.dumps(results, indent=1))
failed = [result for result in results if result["exit"] != 0]
print(f"{len(results)} runs, {len(failed)} failed: {[result['log'] for result in failed]}")
