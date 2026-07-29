#!/usr/bin/env python3
import http.server
import signal
import socketserver
import sys


class Server(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # Avoid HTTPServer's unnecessary FQDN lookup for a loopback fixture.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


signal.signal(signal.SIGUSR1, signal.SIG_IGN)
port = int(sys.argv[1])
with Server(("127.0.0.1", port), http.server.SimpleHTTPRequestHandler) as server:
    server.serve_forever()
