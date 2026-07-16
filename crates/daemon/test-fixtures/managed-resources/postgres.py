#!/usr/bin/env python3
import os
import signal
import socketserver
import struct
import sys
import threading


arguments = list(sys.argv[1:])
data_dir = ""
argument_host = ""
argument_port = ""

while arguments:
    argument = arguments.pop(0)
    if argument == "-D":
        data_dir = arguments.pop(0)
    elif argument == "-h":
        argument_host = arguments.pop(0)
    elif argument == "-p":
        argument_port = arguments.pop(0)
    else:
        print(f"unexpected postgres argument: {argument}", file=sys.stderr)
        sys.exit(64)

if (
    not data_dir
    or not argument_host
    or not argument_port
    or not os.path.isfile(os.path.join(data_dir, "PG_VERSION"))
):
    print("postgres data dir is not initialized", file=sys.stderr)
    sys.exit(64)

argument_port = int(argument_port)
config_path = os.path.join(data_dir, "postgresql.conf")
database_dir = os.path.join(data_dir, "databases")

host = "127.0.0.1"
port = None

with open(config_path, "r", encoding="utf-8") as config:
    for line in config:
        line = line.strip()
        if line.startswith("listen_addresses"):
            host = line.split("=", 1)[1].strip().strip("'\"")
        if line.startswith("port"):
            port = int(line.split("=", 1)[1].strip())

if host != "127.0.0.1" or port is None:
    raise SystemExit("postgresql.conf did not set loopback host and port")
if argument_host != host or argument_port != port:
    raise SystemExit("postgres arguments did not match generated config")

os.makedirs(database_dir, exist_ok=True)
with open(
    os.path.join(data_dir, "postgres.started"), "w", encoding="utf-8"
) as started:
    started.write(f"{host}:{port}\n")


def packet(message_type, payload=b""):
    return message_type + struct.pack("!I", len(payload) + 4) + payload


def auth_ok():
    return packet(b"R", struct.pack("!I", 0))


def parameter_status(key, value):
    return packet(b"S", key.encode() + b"\0" + value.encode() + b"\0")


def backend_key_data():
    return packet(b"K", struct.pack("!II", os.getpid() & 0x7FFFFFFF, 1))


def ready():
    return packet(b"Z", b"I")


def parameter_description(query):
    if "$1" in query:
        return packet(b"t", struct.pack("!H", 1) + struct.pack("!I", 25))
    return packet(b"t", struct.pack("!H", 0))


def command_complete(tag):
    return packet(b"C", tag.encode() + b"\0")


def parse_complete():
    return packet(b"1")


def bind_complete():
    return packet(b"2")


def close_complete():
    return packet(b"3")


def no_data():
    return packet(b"n")


def row_description():
    field = b"?column?\0" + struct.pack("!IhIhih", 0, 0, 23, 4, -1, 0)
    return packet(b"T", struct.pack("!H", 1) + field)


def data_row(value):
    data = str(value).encode()
    return packet(b"D", struct.pack("!H", 1) + struct.pack("!I", len(data)) + data)


def error_response(message):
    return packet(b"E", b"SERROR\0CXX000\0M" + message.encode() + b"\0\0")


def cstring(payload, start):
    end = payload.index(b"\0", start)
    return payload[start:end].decode(), end + 1


def read_exact(stream, length):
    data = b""
    while len(data) < length:
        chunk = stream.recv(length - len(data))
        if not chunk:
            raise EOFError
        data += chunk
    return data


def read_startup(stream):
    length = struct.unpack("!I", read_exact(stream, 4))[0]
    payload = read_exact(stream, length - 4)
    code = struct.unpack("!I", payload[:4])[0]
    if code == 80877103:
        stream.sendall(b"N")
        return read_startup(stream)
    return payload


def startup_response():
    return b"".join(
        [
            auth_ok(),
            parameter_status("server_version", "16.0"),
            parameter_status("server_encoding", "UTF8"),
            parameter_status("client_encoding", "UTF8"),
            parameter_status("DateStyle", "ISO, MDY"),
            parameter_status("integer_datetimes", "on"),
            parameter_status("standard_conforming_strings", "on"),
            backend_key_data(),
            ready(),
        ]
    )


def database_file(database):
    safe = "".join(character for character in database if character.isalnum() or character == "_")
    if safe != database:
        raise ValueError("unsafe database name")
    return os.path.join(database_dir, database)


def database_exists(database):
    return os.path.exists(database_file(database))


def create_database(database):
    with open(database_file(database), "w", encoding="utf-8") as marker:
        marker.write(database + "\n")


def database_from_create(query):
    quoted = query.split("CREATE DATABASE", 1)[1].strip()
    if quoted.startswith('"') and quoted.endswith('"'):
        return quoted[1:-1]
    return quoted


def query_response(query, params):
    normalized = " ".join(query.strip().split())
    if normalized.upper() in {"SELECT 1", "SELECT $1"}:
        return row_description() + data_row(1) + command_complete("SELECT 1")
    if "FROM pg_database WHERE datname" in normalized:
        database = params[0] if params else ""
        if database_exists(database):
            return row_description() + data_row(1) + command_complete("SELECT 1")
        return row_description() + command_complete("SELECT 0")
    if normalized.upper().startswith("CREATE DATABASE"):
        create_database(database_from_create(normalized))
        return command_complete("CREATE DATABASE")
    if normalized.upper().startswith("SET "):
        return command_complete("SET")
    return error_response("unsupported fixture query: " + normalized)


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        handler_marker = os.environ.get("PV_FIXTURE_HANDLER_STARTED")
        if handler_marker:
            with open(handler_marker, "w", encoding="utf-8") as marker:
                marker.write("started\n")
        statements = {}
        portals = {}
        try:
            read_startup(self.request)
            self.request.sendall(startup_response())
            while True:
                message_type = read_exact(self.request, 1)
                length = struct.unpack("!I", read_exact(self.request, 4))[0]
                payload = read_exact(self.request, length - 4)
                if message_type == b"X":
                    return
                if message_type == b"Q":
                    query = payload[:-1].decode()
                    self.request.sendall(query_response(query, []) + ready())
                    continue
                if message_type == b"P":
                    statement, offset = cstring(payload, 0)
                    query, _offset = cstring(payload, offset)
                    statements[statement] = query
                    self.request.sendall(parse_complete())
                    continue
                if message_type == b"B":
                    portal, offset = cstring(payload, 0)
                    statement, offset = cstring(payload, offset)
                    format_count = struct.unpack("!H", payload[offset : offset + 2])[0]
                    offset += 2 + (format_count * 2)
                    param_count = struct.unpack("!H", payload[offset : offset + 2])[0]
                    offset += 2
                    params = []
                    for _index in range(param_count):
                        size = struct.unpack("!i", payload[offset : offset + 4])[0]
                        offset += 4
                        if size == -1:
                            params.append(None)
                        else:
                            params.append(payload[offset : offset + size].decode())
                            offset += size
                    portals[portal] = (statements.get(statement, ""), params)
                    self.request.sendall(bind_complete())
                    continue
                if message_type == b"D":
                    describe_kind = payload[:1]
                    name = payload[1:-1].decode()
                    query, _params = portals.get(name, (statements.get(name, ""), []))
                    response = b""
                    if describe_kind == b"S":
                        response += parameter_description(query)
                    if query.strip().upper().startswith("CREATE DATABASE"):
                        response += no_data()
                    else:
                        response += row_description()
                    self.request.sendall(response)
                    continue
                if message_type == b"E":
                    portal, offset = cstring(payload, 0)
                    _max_rows = struct.unpack("!I", payload[offset : offset + 4])[0]
                    query, params = portals.get(portal, ("", []))
                    self.request.sendall(query_response(query, params))
                    continue
                if message_type == b"S":
                    self.request.sendall(ready())
                    continue
                if message_type == b"H":
                    continue
                if message_type == b"C":
                    self.request.sendall(close_complete())
                    continue
                self.request.sendall(error_response("unsupported message type"))
        except (EOFError, ConnectionResetError, BrokenPipeError):
            return


class Server(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


server = Server((host, port), Handler)
shutdown_requested = threading.Event()
shutdown_thread = None


def stop(_signum, _frame):
    global shutdown_thread
    if not shutdown_requested.is_set():
        shutdown_requested.set()
        shutdown_thread = threading.Thread(target=server.shutdown, daemon=True)
        shutdown_thread.start()


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

server.serve_forever()
if shutdown_thread is not None:
    shutdown_thread.join()
server.server_close()
