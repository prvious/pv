#!/usr/bin/env python3
import http.server
import os
import socketserver
import sys
import threading
import time


parent_pid = os.getppid()
if parent_pid == 1:
    os._exit(0)


def monitor_parent():
    while True:
        time.sleep(0.1)
        if os.getppid() != parent_pid:
            os._exit(0)


threading.Thread(target=monitor_parent, daemon=True).start()


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        try:
            self.send_response(200)
            self.end_headers()
        finally:
            # Exit even if the readiness client disconnects after reading the status.
            os._exit(0)

    def log_message(self, _format, *_args):
        pass


class Server(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # Avoid HTTPServer's unnecessary FQDN lookup for a loopback fixture.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


server = Server(("127.0.0.1", int(sys.argv[2])), Handler)
server.serve_forever()
