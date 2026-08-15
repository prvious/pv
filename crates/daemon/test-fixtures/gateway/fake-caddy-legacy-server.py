#!/usr/bin/env python3
import http.server
import re
import socketserver
import sys


with open(sys.argv[1], encoding="utf-8") as config_file:
    config = config_file.read()

match = re.search(r"^\s*http_port (\d+)$", config, re.MULTILINE)
if not match:
    raise SystemExit("missing fake Caddy legacy service setting")
port = int(match.group(1))


class Server(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # Avoid HTTPServer's unnecessary FQDN lookup for a loopback fixture.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


with Server(("127.0.0.1", port), http.server.SimpleHTTPRequestHandler) as server:
    server.serve_forever()
