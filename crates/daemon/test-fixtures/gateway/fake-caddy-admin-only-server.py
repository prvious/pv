#!/usr/bin/env python3
import http.server
import os
import re
import socketserver
import sys


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
