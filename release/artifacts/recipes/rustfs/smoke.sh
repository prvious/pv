#!/bin/sh
set -eu

artifact_root=$1
rustfs_binary="$artifact_root/bin/rustfs"
expected_version=${PV_UPSTREAM_VERSION:-}

need() {
  command -v "$1" >/dev/null 2>&1 || {
    printf '%s\n' "missing required command: $1" >&2
    exit 42
  }
}

available_port() {
  python3 - <<'PY'
import socket

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

[ -x "$rustfs_binary" ] || {
  printf '%s\n' "missing executable bin/rustfs in $artifact_root" >&2
  exit 42
}
[ -n "$expected_version" ] || {
  printf '%s\n' "PV_UPSTREAM_VERSION is required for RustFS smoke" >&2
  exit 42
}

need grep
need mktemp
need python3

"$rustfs_binary" --version | grep -F "rustfs $expected_version" >/dev/null

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/pv-rustfs-smoke.XXXXXX")
data_dir="$tmp_dir/data"
log_file="$tmp_dir/rustfs.log"
api_port=$(available_port)
access_key=pvsmokeaccess
secret_key=pvsmokesecret1234567890
bucket=pv-smoke-bucket
mkdir -p "$data_dir"

"$rustfs_binary" server \
  --address "127.0.0.1:$api_port" \
  --access-key "$access_key" \
  --secret-key "$secret_key" \
  "$data_dir" >"$log_file" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; rm -rf "$tmp_dir"' 0

endpoint="http://127.0.0.1:$api_port"
export PV_RUSTFS_SMOKE_ENDPOINT="$endpoint"
export PV_RUSTFS_SMOKE_ACCESS_KEY="$access_key"
export PV_RUSTFS_SMOKE_SECRET_KEY="$secret_key"
export PV_RUSTFS_SMOKE_BUCKET="$bucket"

python3 <<'PY'
import datetime
import hashlib
import hmac
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

endpoint = os.environ["PV_RUSTFS_SMOKE_ENDPOINT"].rstrip("/")
access_key = os.environ["PV_RUSTFS_SMOKE_ACCESS_KEY"]
secret_key = os.environ["PV_RUSTFS_SMOKE_SECRET_KEY"]
bucket = os.environ["PV_RUSTFS_SMOKE_BUCKET"]
region = "us-east-1"
service = "s3"

def sign(key, message):
    return hmac.new(key, message.encode("utf-8"), hashlib.sha256).digest()

def signing_key(date_stamp):
    key = ("AWS4" + secret_key).encode("utf-8")
    key = sign(key, date_stamp)
    key = sign(key, region)
    key = sign(key, service)
    return sign(key, "aws4_request")

def request(method, path):
    parsed = urllib.parse.urlparse(endpoint)
    host = parsed.netloc
    now = datetime.datetime.utcnow()
    amz_date = now.strftime("%Y%m%dT%H%M%SZ")
    date_stamp = now.strftime("%Y%m%d")
    payload_hash = hashlib.sha256(b"").hexdigest()
    canonical_headers = (
        f"host:{host}\n"
        f"x-amz-content-sha256:{payload_hash}\n"
        f"x-amz-date:{amz_date}\n"
    )
    signed_headers = "host;x-amz-content-sha256;x-amz-date"
    canonical_request = "\n".join([
        method,
        path,
        "",
        canonical_headers,
        signed_headers,
        payload_hash,
    ])
    credential_scope = f"{date_stamp}/{region}/{service}/aws4_request"
    string_to_sign = "\n".join([
        "AWS4-HMAC-SHA256",
        amz_date,
        credential_scope,
        hashlib.sha256(canonical_request.encode("utf-8")).hexdigest(),
    ])
    signature = hmac.new(
        signing_key(date_stamp),
        string_to_sign.encode("utf-8"),
        hashlib.sha256,
    ).hexdigest()
    authorization = (
        "AWS4-HMAC-SHA256 "
        f"Credential={access_key}/{credential_scope}, "
        f"SignedHeaders={signed_headers}, Signature={signature}"
    )
    req = urllib.request.Request(
        endpoint + path,
        method=method,
        headers={
            "Authorization": authorization,
            "x-amz-content-sha256": payload_hash,
            "x-amz-date": amz_date,
        },
    )
    with urllib.request.urlopen(req, timeout=3) as response:
        return response.status, response.read()

last_error = None
for _ in range(30):
    try:
        status, _body = request("GET", "/")
        if 200 <= status < 300:
            break
    except (OSError, urllib.error.URLError, urllib.error.HTTPError) as error:
        last_error = error
        time.sleep(1)
else:
    print(f"RustFS S3 API readiness failed: {last_error}", file=sys.stderr)
    sys.exit(43)

status, _body = request("PUT", f"/{bucket}")
if not 200 <= status < 300:
    print(f"RustFS create bucket returned HTTP {status}", file=sys.stderr)
    sys.exit(44)

status, body = request("GET", "/")
if not 200 <= status < 300 or bucket.encode("utf-8") not in body:
    print("RustFS list buckets did not include smoke bucket", file=sys.stderr)
    sys.exit(45)
PY
