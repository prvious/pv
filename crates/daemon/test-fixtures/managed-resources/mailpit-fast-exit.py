#!/usr/bin/env python3
import http.server
import os
import sys


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        print("mailpit fast-exit fixture: request received", file=sys.stderr, flush=True)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ready")
        self.wfile.flush()
        os._exit(0)

    def log_message(self, _format, *_args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[2])), Handler)
print("mailpit fast-exit fixture: server constructed", file=sys.stderr, flush=True)
print("mailpit fast-exit fixture: entering serve loop", file=sys.stderr, flush=True)
server.serve_forever()
