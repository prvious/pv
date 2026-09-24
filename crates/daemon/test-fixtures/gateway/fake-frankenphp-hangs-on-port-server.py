#!/usr/bin/env python3
import http.server
import os
import socketserver
import sys
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


threading.Thread(target=monitor_parent, daemon=True).start()


class Server(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # Avoid HTTPServer's unnecessary FQDN lookup for a loopback fixture.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


port = int(sys.argv[1])
with Server(("127.0.0.1", port), http.server.SimpleHTTPRequestHandler) as server:
    server.serve_forever()
