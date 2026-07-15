#!/usr/bin/env python3
import http.server
import re
import signal
import ssl
import sys
import threading


signal.signal(signal.SIGUSR1, signal.SIG_IGN)

config = open(sys.argv[1], encoding="utf-8").read()


def required(pattern):
    match = re.search(pattern, config, re.MULTILINE)
    if not match:
        raise SystemExit(f"missing fake runtime setting: {pattern}")
    return match.group(1)


def optional(pattern):
    match = re.search(pattern, config, re.MULTILINE)
    if not match:
        return None
    return match.group(1)


class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format, *args):
        pass


http_port = int(required(r"^# PV_FAKE_PORT (\d+)$"))
https_port = optional(r"^\s*https_port (\d+)$")
cert_path = optional(r'^\s*cert "([^"]+)"$')
key_path = optional(r'^\s*key "([^"]+)"$')
servers = [http.server.ThreadingHTTPServer(("127.0.0.1", http_port), Handler)]

if https_port is not None and cert_path is not None and key_path is not None:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certfile=cert_path, keyfile=key_path)
    https_server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", int(https_port)), Handler
    )
    https_server.socket = context.wrap_socket(https_server.socket, server_side=True)
    servers.append(https_server)

for server in servers[1:]:
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

with servers[0] as server:
    server.serve_forever()
