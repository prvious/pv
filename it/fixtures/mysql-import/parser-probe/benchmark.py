#!/usr/bin/env python3
"""Measure the candidate parser's large single-row INSERT memory ceiling."""

import pathlib
import resource
import subprocess
import sys
import tempfile
import time


binary = pathlib.Path(sys.argv[1]).resolve()
literal_mebibytes = int(sys.argv[2]) if len(sys.argv) > 2 else 16
with tempfile.TemporaryDirectory(prefix="pv350-parser-") as temporary:
    dump = pathlib.Path(temporary) / "large.sql"
    with dump.open("wb") as output:
        output.write(b"INSERT INTO `admin`.`blobs` VALUES (1, X'")
        output.write(b"ab" * (literal_mebibytes * 1024 * 1024 // 2))
        output.write(b"');\n")
    started = time.monotonic()
    result = subprocess.run(
        [str(binary), "--squonk", str(dump)],
        capture_output=True,
        text=True,
        check=False,
    )
    elapsed = time.monotonic() - started
    if result.returncode:
        print(result.stderr, file=sys.stderr)
        sys.exit(result.returncode)
    print(f"input_bytes={dump.stat().st_size}")
    print(f"elapsed_seconds={elapsed:.2f}")
    print(f"max_rss_kib={resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss}")
    print(result.stdout)
