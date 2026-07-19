#!/usr/bin/env python3
import http.server
import os
import signal
import socketserver
import sys
import threading


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


class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


smtp_server = TcpServer(host_port(smtp), SmtpHandler)
dashboard = http.server.ThreadingHTTPServer(
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
