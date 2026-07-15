#!/usr/bin/env python3
import http.server
import signal
import sys


signal.signal(signal.SIGUSR1, signal.SIG_IGN)
port = int(sys.argv[1])
with http.server.ThreadingHTTPServer(
    ("127.0.0.1", port), http.server.SimpleHTTPRequestHandler
) as server:
    server.serve_forever()
