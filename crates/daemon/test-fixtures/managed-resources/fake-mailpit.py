#!/usr/bin/env python3
import errno
import http.server
import os
import signal
import socketserver
import sys
import threading
import time


smtp_port = sys.argv[1]
dashboard_port = sys.argv[2]


class SmtpHandler(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.sendall(b"220 fake mailpit\r\n")


class HttpServer(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # Avoid HTTPServer's unnecessary FQDN lookup for a loopback fixture.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


class HttpHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200 if self.path == "/ready" else 404)
        self.end_headers()

    def log_message(self, _format, *_args):
        pass


class TcpServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


def bind_servers():
    while True:
        smtp_server = None
        try:
            smtp_server = TcpServer(("127.0.0.1", int(smtp_port)), SmtpHandler)
            dashboard_server = HttpServer(
                ("127.0.0.1", int(dashboard_port)),
                HttpHandler,
            )
            return smtp_server, dashboard_server
        except OSError as error:
            if smtp_server is not None:
                smtp_server.server_close()
            if error.errno != errno.EADDRINUSE:
                raise
            time.sleep(0.05)


smtp, dashboard = bind_servers()
shutdown_requested = threading.Event()
shutdown_thread = None
received_signal = None


def shutdown_servers():
    smtp.shutdown()
    dashboard.shutdown()


def stop(signum, _frame):
    global received_signal, shutdown_thread
    if shutdown_requested.is_set():
        return
    received_signal = signum
    shutdown_requested.set()
    shutdown_thread = threading.Thread(target=shutdown_servers, daemon=True)
    shutdown_thread.start()


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

threading.Thread(
    target=smtp.serve_forever, kwargs={"poll_interval": 0.1}, daemon=True
).start()
dashboard.serve_forever(poll_interval=0.1)
if shutdown_thread is not None:
    shutdown_thread.join()
else:
    smtp.shutdown()
smtp.server_close()
dashboard.server_close()
if received_signal is not None:
    signal.signal(received_signal, signal.SIG_DFL)
    os.kill(os.getpid(), received_signal)
