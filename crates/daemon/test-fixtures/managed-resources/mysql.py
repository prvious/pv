#!/usr/bin/env python3
import os
import signal
import socketserver
import sys
import threading


arguments = list(sys.argv[1:])
first_argument = arguments[0] if arguments else ""
data_dir = ""
port = ""
initialize = False

while arguments:
    argument = arguments.pop(0)
    if argument == "--initialize-insecure":
        initialize = True
    elif argument == "--datadir":
        data_dir = arguments.pop(0)
    elif argument == "--basedir":
        arguments.pop(0)
    elif argument == "--port":
        port = arguments.pop(0)
    elif argument in {"--bind-address", "--socket"}:
        arguments.pop(0)
    elif argument == "--no-defaults":
        pass

if initialize:
    if first_argument != "--no-defaults":
        print("mysqld initialization must start with --no-defaults", file=sys.stderr)
        sys.exit(64)
    os.makedirs(f"{data_dir}/mysql", exist_ok=True)
    sys.exit(0)


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        handler_marker = os.environ.get("PV_FIXTURE_HANDLER_STARTED")
        if handler_marker:
            with open(handler_marker, "w", encoding="utf-8") as marker:
                marker.write("started\n")
        self.request.recv(1024)


class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


server = TcpServer(("127.0.0.1", int(port)), Handler)
shutdown_requested = threading.Event()
shutdown_thread = None
received_signal = None


def stop(signum, _frame):
    global received_signal, shutdown_thread
    if not shutdown_requested.is_set():
        received_signal = signum
        shutdown_requested.set()
        shutdown_thread = threading.Thread(target=server.shutdown, daemon=True)
        shutdown_thread.start()


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

server.serve_forever()
if shutdown_thread is not None:
    shutdown_thread.join()
server.server_close()
if received_signal is not None:
    signal.signal(received_signal, signal.SIG_DFL)
    os.kill(os.getpid(), received_signal)
