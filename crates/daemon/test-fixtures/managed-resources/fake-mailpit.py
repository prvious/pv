#!/usr/bin/env python3
import http.server
import os
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


smtp = TcpServer(("127.0.0.1", int(smtp_port)), SmtpHandler)
dashboard = http.server.ThreadingHTTPServer(
    ("127.0.0.1", int(dashboard_port)),
    http.server.SimpleHTTPRequestHandler,
)
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
