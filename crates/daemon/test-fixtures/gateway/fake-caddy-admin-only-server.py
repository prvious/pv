#!/usr/bin/env python3
import http.server
import re
import socketserver
import sys


with open(sys.argv[1], encoding="utf-8") as config_file:
    config = config_file.read()

match = re.search(r"^\s*admin 127\.0\.0\.1:(\d+)$", config, re.MULTILINE)
if not match:
    raise SystemExit("missing fake Caddy admin setting")
admin_port = int(match.group(1))


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


class Server(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # Avoid HTTPServer's unnecessary FQDN lookup for a loopback fixture.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


with Server(("127.0.0.1", admin_port), Handler) as server:
    server.serve_forever()
