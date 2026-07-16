#!/usr/bin/env python3
import http.server
import signal
import socketserver
import sys
import threading


smtp_port = sys.argv[1]
dashboard_port = sys.argv[2]


class SmtpHandler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.sendall(b"220 fake mailpit\r\n")


class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


def stop(_signum, _frame):
    sys.exit(0)


smtp = TcpServer(("127.0.0.1", int(smtp_port)), SmtpHandler)
dashboard = http.server.ThreadingHTTPServer(
    ("127.0.0.1", int(dashboard_port)),
    http.server.SimpleHTTPRequestHandler,
)

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(target=smtp.serve_forever, daemon=True).start()
dashboard.serve_forever()
