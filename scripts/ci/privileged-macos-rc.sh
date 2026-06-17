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

preserve_rc_binary() {
  install -d "$(dirname "$PV_RC_BIN")" && install -m 755 "$HOME/.pv/bin/pv" "$PV_RC_BIN"
}

record_status environment evidence sw_vers
record_status download-installer required curl --fail --show-error --silent --location --retry 3 --retry-delay 2 "$RESOLVED_INSTALLER_URL" -o "$PV_RC_INSTALLER"
record_status install-pv required bash "$PV_RC_INSTALLER" --no-setup --no-path --non-interactive
export PATH="$HOME/.pv/bin:$PATH"
record_status preserve-rc-binary required preserve_rc_binary
record_status sudo-preflight required sudo -n true || record_blocked sudo-required "passwordless sudo is unavailable on this runner"

mkdir -p "$PV_RC_PROJECT/public"
printf '%s\n' "<?php echo 'pv-privileged-rc-ok';" > "$PV_RC_PROJECT/public/index.php"
printf '%s\n' "document_root: public" > "$PV_RC_PROJECT/pv.yml"
cat > "$PV_RC_EVIDENCE_DIR/checklist.txt" <<'CHECKLIST'
Privileged macOS RC evidence checklist:
- candidate install.sh downloaded and used to install PV
- /etc/resolver/test installed and removed
- pf redirect rules installed and removed
- System keychain CA trust installed and removed
- LaunchAgent installed, printed, restarted, and uninstalled
- Project linked and served through .test
- Update check and diagnostics executed
CHECKLIST

record_status setup required pv setup --yes --no-path
collect_file gateway-caddyfile "$HOME/.pv/config/gateway/Caddyfile"
collect_file gateway-runtime-pid "$HOME/.pv/run/gateway.pid"
collect_file gateway-runtime-metadata "$HOME/.pv/run/gateway.json"
record_status gateway-listeners evidence sh -c 'lsof -nP -iTCP:48080 -sTCP:LISTEN || true; netstat -anv -p tcp | grep -E "48080|48443" || true'
record_status gateway-loopback-nc evidence nc -vz -G 2 127.0.0.1 48080
record_status gateway-loopback-http evidence curl --show-error --silent --max-time 5 --write-out '\nhttp_code:%{http_code}\n' http://127.0.0.1:48080/

collect_file resolver-system /etc/resolver/test
record_status resolver-status required pv dns:status
record_status pf-rules evidence sudo pfctl -sr
record_status pf-nat-rules evidence sudo pfctl -s nat
collect_file pf-anchor /etc/pf.anchors/com.prvious.pv
record_status ports-status required pv ports:status
record_status ca-status required pv ca:status
record_status ca-verify evidence security verify-cert -c "$HOME/.pv/certificates/ca.pem" -p ssl -L
collect_file launch-agent-plist "$HOME/Library/LaunchAgents/com.prvious.pv.daemon.plist"
record_status launch-agent-print required launchctl print "gui/$(id -u)/com.prvious.pv.daemon"

record_status link required pv link "$PV_RC_PROJECT"
record_status link-reconciliation-idle required wait_for_pv_jobs_idle
record_status status-json required pv status --json
record_status serve-http required curl --fail --show-error --silent --location --retry 6 --retry-delay 2 --cacert "$HOME/.pv/certificates/ca.pem" http://pv-rc-project.test/ && require_output_contains serve-http pv-privileged-rc-ok
record_status serve-https required curl --fail --show-error --silent --retry 6 --retry-delay 2 --cacert "$HOME/.pv/certificates/ca.pem" https://pv-rc-project.test/ && require_output_contains serve-https pv-privileged-rc-ok
record_status daemon-restart required pv daemon:restart
record_status restart-reconciliation-idle required wait_for_pv_jobs_idle
record_status post-restart-status-json required pv status --json
record_status post-restart-serve-http required curl --fail --show-error --silent --location --retry 6 --retry-delay 2 --cacert "$HOME/.pv/certificates/ca.pem" http://pv-rc-project.test/ && require_output_contains post-restart-serve-http pv-privileged-rc-ok
record_status post-restart-serve-https required curl --fail --show-error --silent --retry 6 --retry-delay 2 --cacert "$HOME/.pv/certificates/ca.pem" https://pv-rc-project.test/ && require_output_contains post-restart-serve-https pv-privileged-rc-ok
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
