#!/usr/bin/env python3
import http.server
import glob
import json
import os
import pathlib
import re
import socketserver
import ssl
import sys
import threading
import time


config_path = pathlib.Path(sys.argv[1])
state_directory = config_path.parent
control_path = state_directory / "fake-admin-control.json"
current_path = state_directory / "fake-admin-current.bin"
request_log_path = state_directory / "fake-admin-requests.jsonl"
runtime_name = os.environ.get("PV_FAKE_RUNTIME", "runtime")


def read_config(path):
    return path.read_bytes()


def setting(pattern, config):
    match = re.search(pattern, config.decode("utf-8"), re.MULTILINE)
    if not match:
        raise SystemExit(f"missing fake {runtime_name} setting: {pattern}")
    return match.group(1)


def optional_setting(pattern, config):
    match = re.search(pattern, config.decode("utf-8"), re.MULTILINE)
    if not match:
        return None
    return match.group(1)


class RuntimeState:
    def __init__(self, initial_config):
        self.lock = threading.Lock()
        self.current_config = initial_config
        self.load_number = 0
        current_path.write_bytes(initial_config)

    def control(self):
        try:
            return json.loads(control_path.read_text(encoding="utf-8"))
        except FileNotFoundError:
            return {}
        except (OSError, json.JSONDecodeError):
            return {}

    def consume_value(self, name, default):
        with self.lock:
            control = self.control()
            value = control.get(name, default)
            if isinstance(value, list):
                if not value:
                    return default
                next_value = value.pop(0)
                control[name] = value
                try:
                    control_path.write_text(json.dumps(control), encoding="utf-8")
                except OSError:
                    pass
                return next_value
            return value
    def consume_status(self, name):
        with self.lock:
            control = self.control()
            values = control.get(name, [])
            if not isinstance(values, list) or not values:
                return 200
            value = values.pop(0)
            try:
                status = int(value)
            except (TypeError, ValueError):
                status = 500
            try:
                control[name] = values
                control_path.write_text(json.dumps(control), encoding="utf-8")
            except OSError:
                pass
            return status

    def setting(self, name, default):
        value = self.control().get(name, default)
        return value

    def record_request(self, method, path, status, body=b""):
        with self.lock:
            try:
                with request_log_path.open("a", encoding="utf-8") as request_log:
                    request_log.write(
                        json.dumps(
                            {
                                "body_length": len(body),
                                "method": method,
                                "path": path,
                                "status": status,
                            }
                        )
                        + "\n"
                    )
            except OSError:
                pass

    def record_load(self, body):
        with self.lock:
            body_path = state_directory / f"fake-admin-load-{self.load_number:03}.bin"
            body_path.write_bytes(body)
            self.load_number += 1

    def apply_config(self, body):
        with self.lock:
            self.current_config = body
            current_path.write_bytes(body)


initial_config = read_config(config_path)
state = RuntimeState(initial_config)
http_port = None
http_port_setting = optional_setting(r"^\s*http_port (\d+)$", initial_config)
if http_port_setting is not None:
    http_port = int(http_port_setting)
else:
    import_path = setting(r'^\s*import\s+"([^"]+)"$', initial_config)
    for fragment_path in glob.glob(import_path):
        fragment = pathlib.Path(fragment_path).read_text(encoding="utf-8")
        match = re.search(r"\bhttp://[^\s,]+:(\d+)\b", fragment)
        if match:
            http_port = int(match.group(1))
            break
if http_port is None:
    raise SystemExit(f"missing fake {runtime_name} service port")
admin_port = int(setting(r"^\s*admin 127\.0\.0\.1:(\d+)$", initial_config))
https_port = optional_setting(r"^\s*https_port (\d+)$", initial_config)
cert_path = optional_setting(r'^\s*cert "([^"]+)"$', initial_config)
key_path = optional_setting(r'^\s*key "([^"]+)"$', initial_config)


def health_response(config):
    response = optional_setting(r'\srespond /__pv/health "([^"]+)"', config)
    if response is not None:
        return response.encode("utf-8")
    return f"pv-{runtime_name}-health".encode("utf-8")


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass

    def send_body(self, status, body):
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("X-PV-Fake-Runtime", runtime_name)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/config/":
            status = state.consume_status("admin_statuses")
            state.record_request("GET", self.path, status)
            self.send_body(status, b"{}\n")
            return

        if self.path == "/__pv/health":
            status = state.consume_status("public_readiness_statuses")
            state.record_request("GET", self.path, status)
            self.send_body(
                status,
                health_response(state.current_config),
            )
            return

        state.record_request("GET", self.path, 404)
        self.send_body(404, b"not found\n")

    def do_POST(self):
        if self.path != "/load":
            self.send_body(404, b"not found\n")
            return

        content_length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(content_length)
        status = state.consume_status("load_statuses")
        state.record_request("POST", self.path, status, body)
        state.record_load(body)
        try:
            delay_ms = int(state.consume_value("load_delay_ms", 0))
        except (TypeError, ValueError):
            delay_ms = 0
        late_accept = state.consume_value("late_accept", False)
        try:
            late_apply_delay_ms = int(state.consume_value("late_apply_delay_ms", 0))
        except (TypeError, ValueError):
            late_apply_delay_ms = 0
        if 200 <= status < 300 and late_accept:
            time.sleep(max(0, late_apply_delay_ms) / 1000)
            state.apply_config(body)
            remaining_delay_ms = max(0, delay_ms - late_apply_delay_ms)
            if remaining_delay_ms > 0:
                time.sleep(remaining_delay_ms / 1000)
        else:
            if delay_ms > 0:
                time.sleep(delay_ms / 1000)
            if 200 <= status < 300:
                state.apply_config(body)
        self.send_body(status, b"accepted\n" if 200 <= status < 300 else b"rejected\n")
        if 200 <= status < 300 and state.consume_value("exit_after_load", False):
            threading.Timer(0.05, os._exit, args=(0,)).start()


class Server(http.server.ThreadingHTTPServer):
    allow_reuse_address = True

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


servers = [Server(("127.0.0.1", http_port), Handler)]
admin_server = Server(("127.0.0.1", admin_port), Handler)

if https_port is not None and cert_path is not None and key_path is not None:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certfile=cert_path, keyfile=key_path)
    https_server = Server(("127.0.0.1", int(https_port)), Handler)
    https_server.socket = context.wrap_socket(https_server.socket, server_side=True)
    servers.append(https_server)

for server in [admin_server, *servers[1:]]:
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

with servers[0] as server:
    server.serve_forever()
