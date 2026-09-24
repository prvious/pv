#!/usr/bin/env python3
import os
import signal
import threading
import time


parent_pid = os.getppid()
if parent_pid == 1:
    os._exit(0)
expected_parent_pid = os.environ.get("PV_FAKE_FIXTURE_PARENT_PID")
if expected_parent_pid is None:
    expected_parent_pid = parent_pid
else:
    try:
        expected_parent_pid = int(expected_parent_pid)
    except ValueError:
        expected_parent_pid = parent_pid
if expected_parent_pid == 1:
    os._exit(0)


def raise_system_exit():
    raise SystemExit(0)


def monitor_parent():
    while True:
        current_parent_pid = os.getppid()
        if current_parent_pid == 1 or current_parent_pid != parent_pid:
            os._exit(0)
        if expected_parent_pid != parent_pid:
            try:
                os.kill(expected_parent_pid, 0)
            except OSError:
                os._exit(0)
        time.sleep(0.1)


signal.signal(signal.SIGTERM, lambda _signum, _frame: raise_system_exit())
signal.signal(signal.SIGINT, lambda _signum, _frame: raise_system_exit())
threading.Thread(target=monitor_parent, daemon=True).start()


while True:
    time.sleep(1)
