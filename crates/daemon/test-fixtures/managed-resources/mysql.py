#!/usr/bin/env python3
import os
import signal
import socketserver
import sys


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
        self.request.recv(1024)


class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True


def stop(_signum, _frame):
    sys.exit(0)


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

server = TcpServer(("127.0.0.1", int(port)), Handler)
server.serve_forever()
