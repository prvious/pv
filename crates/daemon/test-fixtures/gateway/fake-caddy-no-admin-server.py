#!/usr/bin/env python3
import signal
import time


signal.signal(signal.SIGTERM, lambda _signum, _frame: raise_system_exit())
signal.signal(signal.SIGINT, lambda _signum, _frame: raise_system_exit())


def raise_system_exit():
    raise SystemExit(0)


while True:
    time.sleep(1)
