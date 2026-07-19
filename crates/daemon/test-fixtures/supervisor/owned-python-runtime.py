#!/usr/bin/env python3
import signal
import sys


def stop(_signum, _frame):
    sys.exit(0)


if sys.argv[1:] != ["1025", "8025"]:
    sys.exit(2)

signal.signal(signal.SIGTERM, stop)
signal.pause()
