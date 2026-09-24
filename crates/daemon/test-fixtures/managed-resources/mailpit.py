#!/usr/bin/env python3
import http.server
import os
import signal
import socketserver
import sys
import threading
import time


parent_pid = os.getppid()
parent_capture_marker = os.environ.get("PV_TEST_PARENT_CAPTURE_MARKER")
parent_capture_release = os.environ.get("PV_TEST_PARENT_CAPTURE_RELEASE")
if parent_capture_marker and parent_capture_release:
    with open(parent_capture_marker, "w", encoding="utf-8") as marker:
        marker.write("started\n")
    while not os.path.exists(parent_capture_release):
        if os.getppid() != parent_pid:
            os._exit(0)
        time.sleep(0.01)
if parent_pid == 1:
    os._exit(0)


def monitor_parent():
    while True:
        time.sleep(0.1)
        if os.getppid() != parent_pid:
            os._exit(0)


threading.Thread(target=monitor_parent, daemon=True).start()


arguments = list(sys.argv[1:])
smtp = ""
listen = ""
database = ""
disable_version_check = False

while arguments:
    argument = arguments.pop(0)
    if argument == "--smtp":
        smtp = arguments.pop(0)
    elif argument == "--listen":
        listen = arguments.pop(0)
    elif argument == "--database":
        database = arguments.pop(0)
    elif argument == "--disable-version-check":
        disable_version_check = True
    else:
        print(f"unexpected argument: {argument}", file=sys.stderr)
        sys.exit(2)

if not smtp or not listen or not database:
    print("missing required mailpit argument", file=sys.stderr)
    sys.exit(2)

if not disable_version_check:
    print("missing --disable-version-check", file=sys.stderr)
    sys.exit(2)

if not database.endswith("/mailpit.db"):
    print(f"unexpected database path: {database}", file=sys.stderr)
    sys.exit(2)

database_dir = os.path.dirname(database)
if not os.path.isdir(database_dir):
    print(f"database directory does not exist: {database_dir}", file=sys.stderr)
    sys.exit(2)


def host_port(value):
    host, port = value.rsplit(":", 1)
    return host, int(port)


class SmtpHandler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.sendall(b"220 mailpit fixture\r\n")


class HttpServer(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # Avoid HTTPServer's unnecessary FQDN lookup for a loopback fixture.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


smtp_server = TcpServer(host_port(smtp), SmtpHandler)
dashboard = HttpServer(
    host_port(listen),
    http.server.SimpleHTTPRequestHandler,
)
shutdown_requested = threading.Event()
shutdown_thread = None
received_signal = None


def shutdown_servers():
    smtp_server.shutdown()
    dashboard.shutdown()


def stop(signum, _frame):
    global received_signal, shutdown_thread
    if shutdown_requested.is_set():
        return
    received_signal = signum
    shutdown_requested.set()
    shutdown_thread = threading.Thread(target=shutdown_servers, daemon=True)
    shutdown_thread.start()


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(
    target=smtp_server.serve_forever, kwargs={"poll_interval": 0.1}, daemon=True
).start()
dashboard.serve_forever(poll_interval=0.1)
if shutdown_thread is not None:
    shutdown_thread.join()
else:
    smtp_server.shutdown()
smtp_server.server_close()
dashboard.server_close()
if received_signal is not None:
    signal.signal(received_signal, signal.SIG_DFL)
    os.kill(os.getpid(), received_signal)
