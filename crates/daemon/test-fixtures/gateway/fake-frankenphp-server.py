#!/usr/bin/env python3
import http.server
import re
import signal
import socketserver
import ssl
import sys
import threading


signal.signal(signal.SIGUSR1, signal.SIG_IGN)

with open(sys.argv[1], encoding="utf-8") as config_file:
    config = config_file.read()


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

    def do_GET(self):
        if self.path == "/__pv/health":
            body = gateway_health_response.encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return

        super().do_GET()


class Server(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # Avoid HTTPServer's unnecessary FQDN lookup for a loopback fixture.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


http_port = int(required(r"^# PV_FAKE_PORT (\d+)$"))
https_port = optional(r"^\s*https_port (\d+)$")
gateway_health_response = f"pv-gateway-health-v1:{http_port}:{https_port}"
cert_path = optional(r'^\s*cert "([^"]+)"$')
key_path = optional(r'^\s*key "([^"]+)"$')
servers = [Server(("127.0.0.1", http_port), Handler)]

if https_port is not None and cert_path is not None and key_path is not None:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certfile=cert_path, keyfile=key_path)
    https_server = Server(("127.0.0.1", int(https_port)), Handler)
    https_server.socket = context.wrap_socket(https_server.socket, server_side=True)
    servers.append(https_server)

for server in servers[1:]:
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

with servers[0] as server:
    server.serve_forever()
