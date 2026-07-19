#!/usr/bin/env python3
import os
import signal
import shlex
import socketserver
import sys
import threading


def redis_config(argv):
    port = None
    data_dir = None
    arguments = list(argv)
    while arguments:
        argument = arguments.pop(0)
        if argument == "--port" and arguments:
            port = int(arguments.pop(0))
        elif argument == "--dir" and arguments:
            data_dir = arguments.pop(0)
        elif os.path.isfile(argument):
            with open(argument, "r", encoding="utf-8") as config:
                for line in config:
                    parts = shlex.split(line)
                    if len(parts) == 2 and parts[0] == "port":
                        port = int(parts[1])
                    elif len(parts) == 2 and parts[0] == "dir":
                        data_dir = parts[1]
    if port is None:
        raise RuntimeError("missing Redis port")
    return port, data_dir


class RedisPingHandler(socketserver.BaseRequestHandler):
    def handle(self):
        while True:
            data = self.request.recv(4096)
            if not data:
                return
            upper = data.upper()
            responses = []
            for _index in range(upper.count(b"CLIENT")):
                responses.append(b"+OK\r\n")
            for _index in range(upper.count(b"PING")):
                responses.append(b"+PONG\r\n")
            if not responses:
                responses.append(b"+OK\r\n")
            self.request.sendall(b"".join(responses))


class RedisServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


shutdown_requested = threading.Event()
shutdown_thread = None
received_signal = None


def stop(signum, _frame):
    global received_signal, shutdown_thread
    if shutdown_requested.is_set():
        return
    received_signal = signum
    shutdown_requested.set()
    shutdown_thread = threading.Thread(target=server.shutdown, daemon=True)
    shutdown_thread.start()


port, data_dir = redis_config(sys.argv[1:])
if data_dir:
    os.makedirs(data_dir, exist_ok=True)

server = RedisServer(("127.0.0.1", port), RedisPingHandler)
signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
server.serve_forever()
if shutdown_thread is not None:
    shutdown_thread.join()
server.server_close()
if received_signal is not None:
    signal.signal(received_signal, signal.SIG_DFL)
    os.kill(os.getpid(), received_signal)
