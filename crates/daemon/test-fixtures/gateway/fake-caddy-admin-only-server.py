#!/usr/bin/env python3
import http.server
import os
import re
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


with open(sys.argv[1], encoding="utf-8") as config_file:
    config = config_file.read()

match = re.search(r'^\s*admin "unix/([^"|]+)\|0600"$', config, re.MULTILINE)
if not match:
    raise SystemExit("missing fake Caddy admin setting")
admin_socket = match.group(1)


class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format, *args):
        pass

    def do_GET(self):
        if self.path != "/config/":
            self.send_error(404)
            return

        body = b"{}\n"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        if self.path != "/load":
            self.send_error(404)
            return

        content_length = int(self.headers.get("Content-Length", "0"))
        self.rfile.read(content_length)
        self.send_response(200)
        self.send_header("Content-Length", "0")
        self.end_headers()


class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


with Server(admin_socket, Handler) as server:
    os.chmod(admin_socket, 0o600)
    server.serve_forever()
