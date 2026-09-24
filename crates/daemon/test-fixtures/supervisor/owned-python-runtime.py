#!/usr/bin/env python3
import os
import signal
import sys
import threading
import time


parent_pid = os.getppid()
if parent_pid == 1:
    os._exit(0)


def monitor_parent():
    while True:
        time.sleep(0.1)
        if os.getppid() != parent_pid:
            os._exit(0)


threading.Thread(target=monitor_parent, daemon=True).start()


def stop(_signum, _frame):
    sys.exit(0)


if sys.argv[1:] != ["1025", "8025"]:
    sys.exit(2)

signal.signal(signal.SIGTERM, stop)
signal.pause()
