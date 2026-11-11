#!/usr/bin/env bash

set -u
set +e

PV_RC_EVIDENCE_DIR="${PV_RC_EVIDENCE_DIR:-${RUNNER_TEMP:?}/pv-privileged-rc-evidence}"
mkdir -p "$PV_RC_EVIDENCE_DIR"

PV_RC_INSTALLER="${RUNNER_TEMP:?}/pv-privileged-rc-install.sh"
PV_RC_BIN="${RUNNER_TEMP:?}/pv-privileged-rc-bin/pv"
PV_RC_PROJECT="${RUNNER_TEMP:?}/pv-rc-project"
failure_count=0

record_blocked() {
  local label=$1
  local reason=$2
  printf 'blocked: %s\n' "$reason" > "$PV_RC_EVIDENCE_DIR/$label.status"
  printf '::error title=%s::%s\n' "$label" "$reason"
  failure_count=$((failure_count + 1))
}

record_status() {
  local label=$1
  local required=$2
  shift 2
  printf 'command: %s\n' "$*" > "$PV_RC_EVIDENCE_DIR/$label.status"
  "$@" > "$PV_RC_EVIDENCE_DIR/$label.out" 2> "$PV_RC_EVIDENCE_DIR/$label.err"
  local status=$?
  printf 'exit_status: %s\n' "$status" >> "$PV_RC_EVIDENCE_DIR/$label.status"
  if [ "$status" -ne 0 ] && [ "$required" = "required" ]; then
    printf '::error title=%s failed::exit status %s\n' "$label" "$status"
    failure_count=$((failure_count + 1))
  fi
  return "$status"
}

require_output_contains() {
  local label=$1
  local expected=$2
  if grep -Fq "$expected" "$PV_RC_EVIDENCE_DIR/$label.out"; then
    return 0
  fi
  printf 'expected: %s\n' "$expected" > "$PV_RC_EVIDENCE_DIR/$label.assertion"
  printf '::error title=%s output missing expected body::%s\n' "$label" "$expected"
  failure_count=$((failure_count + 1))
  return 1
}

collect_file() {
  local label=$1
  local path=$2
  if [ -e "$path" ]; then
    sudo sh -c 'cat "$1" > "$2" 2> "$3"' sh "$path" "$PV_RC_EVIDENCE_DIR/$label.out" "$PV_RC_EVIDENCE_DIR/$label.err"
    printf 'path: %s\nexit_status: %s\n' "$path" "$?" > "$PV_RC_EVIDENCE_DIR/$label.status"
  else
    printf 'missing: %s\n' "$path" > "$PV_RC_EVIDENCE_DIR/$label.status"
  fi
}

pv_pf_rules_absent() {
  local filter_rules
  local nat_rules
  filter_rules=$(sudo pfctl -sr) || return 1
  nat_rules=$(sudo pfctl -s nat) || return 1
  printf 'filter rules:\n%s\nnat rules:\n%s\n' "$filter_rules" "$nat_rules"
  if printf '%s\n%s\n' "$filter_rules" "$nat_rules" | grep -Fq "com.prvious.pv"; then
    return 1
  fi

  return 0
}

# This is probably temporary. We should remember to remove it on the next release.
# Leaves the pf redirect a PV release before launchd sockets installed: its anchor, and its lines
# in /etc/pf.conf, loaded.
seed_legacy_pf() {
  local seeded="$PV_RC_EVIDENCE_DIR/pf.conf.seeded"
  printf '%s\n' '# Managed by PV' \
    'rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 48080' \
    'rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port 48443' |
    sudo tee /etc/pf.anchors/com.prvious.pv > /dev/null || return 1
  awk '{ print }
    /^rdr-anchor "com\.apple\/\*"/ {
      print "# Managed by PV"
      print "# Source: PV prepared pf.conf reference for /etc/pf.conf"
      print "rdr-anchor \"com.prvious.pv\""
    }
    END { print "load anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"" }' \
    /etc/pf.conf > "$seeded" || return 1
  grep -Fxq 'rdr-anchor "com.prvious.pv"' "$seeded" || return 1
  sudo install -m 0644 "$seeded" /etc/pf.conf || return 1
  sudo pfctl -f /etc/pf.conf || return 1
  sudo pfctl -s nat | grep -Fq 'com.prvious.pv'
}

pv_ca_trust_removed() {
  "$PV_RC_BIN" ca:status | grep -F "System keychain trust: not trusted"
}

wait_for_pv_jobs_idle() {
  local deadline=$((SECONDS + 90))
  local jobs_file="$PV_RC_EVIDENCE_DIR/jobs-idle.json"
  local jobs_error="$PV_RC_EVIDENCE_DIR/jobs-idle.err"

  while [ "$SECONDS" -le "$deadline" ]; do
    pv jobs --json > "$jobs_file" 2> "$jobs_error"
    local status=$?
    if [ "$status" -eq 0 ] && python3 -c 'import json, sys; jobs = json.load(open(sys.argv[1], encoding="utf-8")).get("jobs", []); sys.exit(0 if all(job.get("status") != "running" for job in jobs) else 1)' "$jobs_file"; then
      cat "$jobs_file"
      return 0
    fi
    sleep 2
  done

  cat "$jobs_error" >&2
  cat "$jobs_file"
  return 1
}

launch_agent_lists_gateway_sockets() {
  local printed
  printed=$(launchctl print "gui/$(id -u)/com.prvious.pv.daemon") || return 1
  printf '%s\n' "$printed"
  grep -Eq 'GatewayHttp([^s]|$)' <<< "$printed" && grep -Fq GatewayHttps <<< "$printed"
}

ROOT_PORT_80_HOLDER_PID=
ROOT_PORT_80_HOLDER_PID_FILE="$PV_RC_EVIDENCE_DIR/root-port-80-holder.pid"

# Holds 127.0.0.1:80 as root for up to 120 s and records the listener's own pid.
start_root_port_80_holder() {
  sudo python3 -c 'import os, socket, sys, time
listener = socket.socket()
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", 80))
listener.listen()
with open(sys.argv[1] + ".tmp", "w") as pid_file:
    pid_file.write(str(os.getpid()))
os.rename(sys.argv[1] + ".tmp", sys.argv[1])
time.sleep(120)' "$ROOT_PORT_80_HOLDER_PID_FILE" &
  local deadline=$((SECONDS + 10))
  until [ -s "$ROOT_PORT_80_HOLDER_PID_FILE" ]; do
    [ "$SECONDS" -le "$deadline" ] || return 1
    sleep 0.2
  done
  ROOT_PORT_80_HOLDER_PID=$(cat "$ROOT_PORT_80_HOLDER_PID_FILE")
  printf 'pid: %s\n' "$ROOT_PORT_80_HOLDER_PID"
}

stop_root_port_80_holder() {
  sudo kill "$ROOT_PORT_80_HOLDER_PID"
}

gateway_failed_on_port_80() {
  pv status --json > "$PV_RC_EVIDENCE_DIR/root-holder-status.json"
  python3 -c 'import json, sys
runtimes = json.load(open(sys.argv[1], encoding="utf-8"))["runtimes"]
gateway = next(runtime for runtime in runtimes if runtime["subject"] == "gateway")
print(gateway)
sys.exit(0 if gateway["status"] == "failed" and "Port 80" in (gateway["message"] or "") else 1)' "$PV_RC_EVIDENCE_DIR/root-holder-status.json"
}

DNS_PORT_BEFORE_REPAIR=
DNS_PORT_HOLDER_PID=

record_dns_port() {
  DNS_PORT_BEFORE_REPAIR=$(pv dns:status | sed -n 's/^DNS port: \([0-9][0-9]*\)$/\1/p')
  [ -n "$DNS_PORT_BEFORE_REPAIR" ] || return 1
  printf 'port: %s\n' "$DNS_PORT_BEFORE_REPAIR"
}

resolver_port() {
  sed -n 's/^port //p' /etc/resolver/test 2>/dev/null
}

# The daemon repairs /etc/resolver/test on its next health tick, every 30 s.
wait_for_resolver_port() {
  local expected=$1
  local deadline=$((SECONDS + 40))
  until [ "$(resolver_port)" = "$expected" ]; do
    [ "$SECONDS" -le "$deadline" ] || return 1
    sleep 1
  done
  cat /etc/resolver/test
}

wait_for_moved_resolver_port() {
  local deadline=$((SECONDS + 40))
  local port
  while [ "$SECONDS" -le "$deadline" ]; do
    port=$(resolver_port)
    if [ -n "$port" ] && [ "$port" != "$DNS_PORT_BEFORE_REPAIR" ]; then
      cat /etc/resolver/test
      return 0
    fi
    sleep 1
  done
  return 1
}

# Holds PV's DNS port over TCP as the user for up to 120 s.
start_dns_port_holder() {
  python3 -c 'import socket, sys, time
listener = socket.socket()
listener.bind(("127.0.0.1", int(sys.argv[1])))
listener.listen()
time.sleep(120)' "$DNS_PORT_BEFORE_REPAIR" &
  DNS_PORT_HOLDER_PID=$!
  sleep 1
  kill -0 "$DNS_PORT_HOLDER_PID"
}

preserve_rc_binary() {
  install -d "$(dirname "$PV_RC_BIN")" && install -m 755 "$HOME/.pv/bin/pv" "$PV_RC_BIN"
}

resolve_project_tls_cert() {
  local project_tls_certificates=("$HOME"/.pv/certificates/projects/*/tls.crt)
  if [ "${#project_tls_certificates[@]}" -ne 1 ] || [ ! -f "${project_tls_certificates[0]}" ]; then
    printf 'expected exactly one Project TLS certificate leaf, found %s\n' "${#project_tls_certificates[@]}" >&2
    return 1
  fi

  PROJECT_TLS_CERT=${project_tls_certificates[0]}
}

record_project_tls_metadata() {
  openssl x509 \
    -in "$1" \
    -noout \
    -text \
    -subject \
    -issuer \
    -startdate \
    -enddate
}

assert_project_tls_lifetime() {
  local certificate=$1
  if [ -z "$certificate" ]; then
    return 1
  fi

  LC_ALL=C python3 - "$certificate" <<'PY'
import datetime
import subprocess
import sys

output = subprocess.check_output(
    ["openssl", "x509", "-in", sys.argv[1], "-noout", "-startdate", "-enddate"],
    text=True,
)
dates = {}
for line in output.splitlines():
    name, value = line.split("=", 1)
    dates[name] = datetime.datetime.strptime(
        value.strip().rsplit(" ", 1)[0], "%b %d %H:%M:%S %Y"
    )

lifetime = dates["notAfter"] - dates["notBefore"]
print(f"lifetime_seconds={int(lifetime.total_seconds())}")
print(f"lifetime_days={lifetime.total_seconds() / 86400:.6f}")
if lifetime > datetime.timedelta(days=366):
    raise SystemExit("Project TLS certificate lifetime exceeds 366 days")
PY
}

require_binary_contains_url() {
  local label=$1
  local url=$2

  if [ -z "$url" ]; then
    return 0
  fi

  if strings "$PV_RC_BIN" | grep -Fq "$url"; then
    return 0
  fi

  printf 'compiled PV binary does not contain %s URL: %s\n' "$label" "$url" >&2
  return 1
}

assert_helper_installation_contract() {
  local owner_uid
  local owner_gid
  owner_uid=$(id -u) || return 1
  owner_gid=$(id -g) || return 1

  [ "$(sudo stat -f '%u:%g:%Lp' /Library/PrivilegedHelperTools/com.prvious.pv.helper)" = "0:0:755" ] || return 1
  [ "$(sudo stat -f '%u:%g:%Lp' /Library/LaunchDaemons/com.prvious.pv.helper.plist)" = "0:0:644" ] || return 1
  [ "$(sudo stat -f '%u:%g:%Lp' "/Library/Application Support/PV/helper.json")" = "0:0:644" ] || return 1
  [ "$(sudo stat -f '%u:%g:%Lp' "/Library/Application Support/PV")" = "0:0:755" ] || return 1
  [ "$(sudo stat -f '%u:%g:%Lp' /var/run/com.prvious.pv.helper.sock)" = "$owner_uid:$owner_gid:600" ] || return 1

  sudo python3 - "$owner_uid" <<'PY' || return 1
import json
import sys

with open("/Library/Application Support/PV/helper.json", encoding="utf-8") as file:
    metadata = json.load(file)

assert set(metadata) == {"owner_uid", "helper_version", "protocol_version"}
assert metadata["owner_uid"] == int(sys.argv[1])
assert isinstance(metadata["helper_version"], str) and metadata["helper_version"]
assert isinstance(metadata["protocol_version"], int) and metadata["protocol_version"] > 0
PY

  sudo python3 - "$owner_uid" "$owner_gid" <<'PY' || return 1
import plistlib
import sys

with open("/Library/LaunchDaemons/com.prvious.pv.helper.plist", "rb") as file:
    plist = plistlib.load(file)

assert set(plist) == {
    "Label", "ProgramArguments", "Sockets", "KeepAlive", "RunAtLoad", "ProcessType", "StandardErrorPath"
}
assert plist["Label"] == "com.prvious.pv.helper"
assert plist["ProgramArguments"] == [
    "/Library/PrivilegedHelperTools/com.prvious.pv.helper"
]
assert plist["KeepAlive"] is False
assert plist["RunAtLoad"] is False
assert plist["ProcessType"] == "Interactive"
assert plist["StandardErrorPath"] == "/var/log/com.prvious.pv.helper.err.log"
assert set(plist["Sockets"]) == {"Control"}
socket = plist["Sockets"]["Control"]
assert socket == {
    "SockPathName": "/var/run/com.prvious.pv.helper.sock",
    "SockPathOwner": int(sys.argv[1]),
    "SockPathGroup": int(sys.argv[2]),
    "SockPathMode": 0o600,
}
PY
}

helper_transaction_files_absent() {
  local path
  for path in \
    /Library/PrivilegedHelperTools/.com.prvious.pv.helper.candidate \
    /Library/PrivilegedHelperTools/.com.prvious.pv.helper.rollback \
    "/Library/Application Support/PV/.helper.json.candidate" \
    "/Library/Application Support/PV/.helper.json.rollback" \
    /Library/LaunchDaemons/.com.prvious.pv.helper.candidate.plist \
    /Library/LaunchDaemons/.com.prvious.pv.helper.rollback.plist
  do
    sudo test ! -e "$path" || return 1
  done
}

record_status environment evidence sw_vers
record_status download-installer required curl --fail --show-error --silent --location --proto '=https' --proto-redir '=https' --retry 3 --retry-delay 2 "$RESOLVED_INSTALLER_URL" -o "$PV_RC_INSTALLER"
record_status install-pv required bash "$PV_RC_INSTALLER" --no-setup --no-path --non-interactive
export PATH="$HOME/.pv/bin:$PATH"
record_status preserve-rc-binary required preserve_rc_binary
record_status compiled-artifact-manifest required require_binary_contains_url artifact-manifest "$RESOLVED_ARTIFACT_MANIFEST_URL"
record_status compiled-app-update-manifest required require_binary_contains_url app-update-manifest "$RESOLVED_APP_UPDATE_MANIFEST_URL"
record_status sudo-preflight required sudo -n true || {
  record_blocked sudo-required "passwordless sudo is unavailable on this runner"
  exit 1
}

mkdir -p "$PV_RC_PROJECT/public"
printf '%s\n' "<?php echo 'pv-privileged-rc-ok';" > "$PV_RC_PROJECT/public/index.php"
cat > "$PV_RC_PROJECT/pv.yml" <<'YAML'
root: public
env:
  VITE_DEV_SERVER_CERT: "${tls.cert}"
  VITE_DEV_SERVER_KEY: "${tls.key}"
YAML
cat > "$PV_RC_EVIDENCE_DIR/checklist.txt" <<'CHECKLIST'
Privileged macOS RC evidence checklist:
- candidate install.sh downloaded and used to install PV
- /etc/resolver/test installed and removed
- an older PV's pf redirect removed by setup
- System keychain CA trust installed and removed
- root helper executable, launchd registration, metadata, and socket installed and removed
- root helper replacement transaction completed and cleaned its temporary files
- LaunchAgent installed, printed with its GatewayHttp and GatewayHttps sockets, restarted, and uninstalled
- Gateway serves ports 80 and 443 from launchd's sockets
- a root listener on port 80 fails the Gateway with the port named, doctor names its pid, and daemon:restart recovers
- the daemon restores a deleted /etc/resolver/test, and moves to a new DNS port when another program holds it
- Project linked and served through .test
- PV placeholder TLS leaf recorded and verified with macOS system policy without an explicit anchor
- Gateway HTTPS verified through installed system trust without --cacert
- Update check and diagnostics executed
CHECKLIST

# This is probably temporary. We should remember to remove it on the next release.
record_status legacy-pf-seed required seed_legacy_pf
record_status setup required pv setup --yes --no-path
# This is probably temporary. We should remember to remove it on the next release.
record_status legacy-pf-removed-by-setup required pv_pf_rules_absent
# This is probably temporary. We should remember to remove it on the next release.
record_status legacy-pf-anchor-removed-by-setup required test ! -e /etc/pf.anchors/com.prvious.pv
record_status helper-executable required sudo test -x /Library/PrivilegedHelperTools/com.prvious.pv.helper
collect_file helper-launch-daemon /Library/LaunchDaemons/com.prvious.pv.helper.plist
collect_file helper-metadata "/Library/Application Support/PV/helper.json"
record_status helper-socket required sudo test -S /var/run/com.prvious.pv.helper.sock
record_status helper-launchd required sudo launchctl print system/com.prvious.pv.helper
record_status helper-installation-contract required assert_helper_installation_contract
record_status helper-replacement-bootout required sudo launchctl bootout system/com.prvious.pv.helper
record_status helper-replacement-setup required pv setup --yes --no-path
record_status helper-replacement-contract required assert_helper_installation_contract
record_status helper-replacement-transaction-cleanup required helper_transaction_files_absent
collect_file helper-stderr-log /var/log/com.prvious.pv.helper.err.log
collect_file gateway-caddyfile "$HOME/.pv/config/gateway/Caddyfile"
collect_file gateway-runtime-pid "$HOME/.pv/run/gateway.pid"
collect_file gateway-runtime-metadata "$HOME/.pv/run/gateway.json"

collect_file resolver-system /etc/resolver/test
record_status resolver-status required pv dns:status
record_status ca-status required pv ca:status
record_status ca-verify evidence security verify-cert -c "$HOME/.pv/certificates/ca.pem" -p ssl -L
collect_file launch-agent-plist "$HOME/Library/LaunchAgents/com.prvious.pv.daemon.plist"
record_status launch-agent-print required launch_agent_lists_gateway_sockets

record_status link required pv link "$PV_RC_PROJECT"
record_status link-reconciliation-idle required wait_for_pv_jobs_idle
PROJECT_TLS_CERT=
record_status project-tls-path required resolve_project_tls_cert
collect_file project-tls-leaf "$PROJECT_TLS_CERT"
record_status project-tls-metadata evidence record_project_tls_metadata "$PROJECT_TLS_CERT"
record_status project-tls-system-policy required security verify-cert -c "$PROJECT_TLS_CERT" -p ssl -s pv-rc-project.test -L
record_status project-tls-lifetime required assert_project_tls_lifetime "$PROJECT_TLS_CERT"
record_status status-json required pv status --json
record_status serve-http required curl --fail --show-error --silent --location --retry 6 --retry-delay 2 --cacert "$HOME/.pv/certificates/ca.pem" http://pv-rc-project.test/ && require_output_contains serve-http pv-privileged-rc-ok
record_status serve-https required curl --fail --show-error --silent --retry 6 --retry-delay 2 https://pv-rc-project.test/ && require_output_contains serve-https pv-privileged-rc-ok
record_status daemon-restart required pv daemon:restart
record_status restart-reconciliation-idle required wait_for_pv_jobs_idle
record_status post-restart-status-json required pv status --json
record_status post-restart-serve-http required curl --fail --show-error --silent --location --retry 6 --retry-delay 2 --cacert "$HOME/.pv/certificates/ca.pem" http://pv-rc-project.test/ && require_output_contains post-restart-serve-http pv-privileged-rc-ok
record_status post-restart-serve-https required curl --fail --show-error --silent --retry 6 --retry-delay 2 https://pv-rc-project.test/ && require_output_contains post-restart-serve-https pv-privileged-rc-ok
record_status port-80-listener evidence sudo lsof -nP -iTCP:80 -sTCP:LISTEN
record_status port-443-listener evidence sudo lsof -nP -iTCP:443 -sTCP:LISTEN
record_status root-holder-daemon-disable required pv daemon:disable
record_status root-holder-start required start_root_port_80_holder
record_status root-holder-daemon-enable required pv daemon:enable
record_status root-holder-reconciliation-idle required wait_for_pv_jobs_idle
record_status root-holder-gateway-failed required gateway_failed_on_port_80
record_status root-holder-doctor evidence pv doctor
require_output_contains root-holder-doctor "pid $ROOT_PORT_80_HOLDER_PID"
record_status root-holder-stop required stop_root_port_80_holder
record_status root-holder-daemon-restart required pv daemon:restart
record_status root-holder-reconciliation-after-restart required wait_for_pv_jobs_idle
record_status root-holder-serve-https required curl --fail --show-error --silent --retry 6 --retry-delay 2 https://pv-rc-project.test/ && require_output_contains root-holder-serve-https pv-privileged-rc-ok
record_status dns-port-before-repair required record_dns_port
record_status dns-resolver-deleted required sudo rm -f /etc/resolver/test
record_status dns-resolver-repaired required wait_for_resolver_port "$DNS_PORT_BEFORE_REPAIR"
record_status dns-holder-daemon-disable required pv daemon:disable
record_status dns-holder-start required start_dns_port_holder
record_status dns-holder-daemon-enable required pv daemon:enable
record_status dns-port-moved required wait_for_moved_resolver_port
record_status dns-moved-lookup required dscacheutil -q host -a name pv-rc-project.test && require_output_contains dns-moved-lookup "ip_address: 127.0.0.1"
record_status dns-holder-stop required kill "$DNS_PORT_HOLDER_PID"
record_status update-check required pv update --check --json
record_status diagnostics required pv doctor
record_status jobs evidence pv jobs
record_status logs evidence pv logs --all

record_status uninstall required pv uninstall
collect_file resolver-after-uninstall /etc/resolver/test
record_status resolver-removed required test ! -e /etc/resolver/test
record_status pf-rules-after-uninstall evidence sudo pfctl -sr
record_status pf-nat-rules-after-uninstall evidence sudo pfctl -s nat
record_status pf-anchor-removed required test ! -e /etc/pf.anchors/com.prvious.pv
record_status pf-rules-removed required pv_pf_rules_absent
record_status ca-status-after-uninstall evidence "$PV_RC_BIN" ca:status
record_status ca-trust-removed required pv_ca_trust_removed
record_status launch-agent-removed required test ! -e "$HOME/Library/LaunchAgents/com.prvious.pv.daemon.plist"
record_status helper-executable-removed required sudo test ! -e /Library/PrivilegedHelperTools/com.prvious.pv.helper
record_status helper-launch-daemon-removed required sudo test ! -e /Library/LaunchDaemons/com.prvious.pv.helper.plist
record_status helper-metadata-removed required sudo test ! -e "/Library/Application Support/PV/helper.json"
record_status helper-socket-removed required sudo test ! -e /var/run/com.prvious.pv.helper.sock
record_status helper-stderr-log-removed required sudo test ! -e /var/log/com.prvious.pv.helper.err.log
record_status helper-support-directory-removed required sudo test ! -e "/Library/Application Support/PV"

{
  printf 'artifact_manifest_url=%s\n' "$RESOLVED_ARTIFACT_MANIFEST_URL"
  printf 'app_update_manifest_url=%s\n' "$RESOLVED_APP_UPDATE_MANIFEST_URL"
  printf 'installer_url=%s\n' "$RESOLVED_INSTALLER_URL"
  printf 'failure_count=%s\n' "$failure_count"
  find "$PV_RC_EVIDENCE_DIR" -maxdepth 1 -type f -print | sort
} > "$PV_RC_EVIDENCE_DIR/summary.txt"

if [ "$failure_count" -ne 0 ]; then
  exit 1
fi
