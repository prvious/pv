#!/usr/bin/env python3
import http.server
import os
import re
import socketserver
import ssl
import sys
import threading


with open(sys.argv[1], encoding="utf-8") as config_file:
    config = config_file.read()


def required(pattern):
    match = re.search(pattern, config, re.MULTILINE)
    if not match:
        raise SystemExit(f"missing fake Caddy setting: {pattern}")
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
        if self.path == "/config/":
            body = b"{}\n"
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("X-PV-Fake-Runtime", "caddy")
            self.end_headers()
            self.wfile.write(body)
            return

        if self.path == "/__pv/health":
            body = gateway_health_response.encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("X-PV-Fake-Runtime", "caddy")
            self.end_headers()
            self.wfile.write(body)
            return

        super().do_GET()

    def do_POST(self):
        if self.path != "/load":
            self.send_error(404)
            return

        content_length = int(self.headers.get("Content-Length", "0"))
        self.rfile.read(content_length)
        self.send_response(200)
        self.send_header("Content-Length", "0")
        self.send_header("X-PV-Fake-Runtime", "caddy")
        self.end_headers()


class Server(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # Avoid HTTPServer's unnecessary FQDN lookup for a loopback fixture.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


class AdminServer(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


http_port = int(required(r"^\s*http_port (\d+)$"))
admin_socket = required(r'^\s*admin "unix/([^"|]+)\|0600"$')
https_port = optional(r"^\s*https_port (\d+)$")
gateway_health_response = f"pv-gateway-health-v1:{http_port}:{https_port}"
cert_path = optional(r'^\s*cert "([^"]+)"$')
key_path = optional(r'^\s*key "([^"]+)"$')
servers = [Server(("127.0.0.1", http_port), Handler)]
admin_server = AdminServer(admin_socket, Handler)
os.chmod(admin_socket, 0o600)

if https_port is not None and cert_path is not None and key_path is not None:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certfile=cert_path, keyfile=key_path)
    https_server = Server(("127.0.0.1", int(https_port)), Handler)
    https_server.socket = context.wrap_socket(https_server.socket, server_side=True)
    servers.append(https_server)

for server in [admin_server, *servers[1:]]:
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

with servers[0] as server:
    server.serve_forever()
