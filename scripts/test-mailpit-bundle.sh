#!/bin/bash
# test-mailpit-bundle.sh — smoke test for a Mailpit binary.
#
# Verifies that mailpit starts, accepts SMTP connections, receives an
# email, and exposes it via the HTTP API.
#
# Usage:
#     scripts/test-mailpit-bundle.sh <path-to-mailpit-binary>
#
# Used by .github/workflows/build-artifacts.yml against the downloaded
# binary, and locally by passing an extracted CI artifact.
#
# Exit code: 0 = all checks passed; non-zero = at least one failed.

set -euo pipefail

MAILPIT="${1:?usage: $0 <path-to-mailpit-binary>}"
SMTP_PORT="${MAILPIT_TEST_SMTP_PORT:-10025}"
HTTP_PORT="${MAILPIT_TEST_HTTP_PORT:-18025}"

if [ ! -x "$MAILPIT" ]; then
    echo "::error::No executable at $MAILPIT"
    exit 1
fi

WORK_DIR="$(mktemp -d)"
MAILPIT_PID=""

cleanup() {
    local rc=$?
    if [ -n "$MAILPIT_PID" ]; then
        kill "$MAILPIT_PID" 2>/dev/null || true
        wait "$MAILPIT_PID" 2>/dev/null || true
    fi
    if [ "$rc" -ne 0 ] && [ -s "$WORK_DIR/mailpit.log" ]; then
        echo "::group::mailpit.log (script exited $rc)" >&2
        cat "$WORK_DIR/mailpit.log" >&2
        echo "::endgroup::" >&2
    fi
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

# 1. version
echo "== 1. mailpit version =="
MAILPIT_VER=$("$MAILPIT" version 2>&1 || true)
echo "  $MAILPIT_VER"

# 2. start mailpit
echo "== 2. start mailpit =="
"$MAILPIT" \
    --smtp 0.0.0.0:$SMTP_PORT \
    --listen 0.0.0.0:$HTTP_PORT \
    --database "$WORK_DIR/mailpit.db" \
    >"$WORK_DIR/mailpit.log" 2>&1 &
MAILPIT_PID=$!

for i in $(seq 1 30); do
    if curl -sf "http://127.0.0.1:$HTTP_PORT/livez" >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

if ! curl -sf "http://127.0.0.1:$HTTP_PORT/livez" >/dev/null 2>&1; then
    echo "::error::mailpit did not become ready within 30s"
    exit 1
fi
echo "  ✓ accepting connections (SMTP :$SMTP_PORT, HTTP :$HTTP_PORT)"

# 3. send a test email via SMTP
echo "== 3. send test email via SMTP =="
# Use curl's SMTP upload capability to send a raw email.
MESSAGE_ID="pv-ci-test-$(date +%s)@test.local"
cat > "$WORK_DIR/email.txt" <<EOF
From: ci@test.local
To: user@test.local
Subject: pv CI smoke test
Message-ID: <$MESSAGE_ID>
Date: $(date -R 2>/dev/null || date)
Content-Type: text/plain

This is a smoke test email from the pv CI pipeline.
EOF

curl -sf --url "smtp://127.0.0.1:$SMTP_PORT" \
    --mail-from "ci@test.local" \
    --mail-rcpt "user@test.local" \
    --upload-file "$WORK_DIR/email.txt"
echo "  ✓ email sent via SMTP"

# 4. verify email appears in the HTTP API
echo "== 4. verify email via HTTP API =="
# Give mailpit a moment to index the message.
sleep 1

MSG_COUNT=$(curl -sf "http://127.0.0.1:$HTTP_PORT/api/v1/messages" | grep -o '"messages_count":[0-9]*' | cut -d: -f2)
if [ -z "$MSG_COUNT" ] || [ "$MSG_COUNT" -lt 1 ]; then
    echo "::error::Expected at least 1 message in API, got '$MSG_COUNT'"
    exit 1
fi
echo "  ✓ $MSG_COUNT message(s) in inbox"

# Verify our specific message by searching for the subject.
SEARCH_HITS=$(curl -sf "http://127.0.0.1:$HTTP_PORT/api/v1/search?query=subject%3A%22pv+CI+smoke+test%22" | grep -o '"messages_count":[0-9]*' | cut -d: -f2)
if [ -z "$SEARCH_HITS" ] || [ "$SEARCH_HITS" -lt 1 ]; then
    echo "::error::Search for test email returned '$SEARCH_HITS' hits, expected >=1"
    exit 1
fi
echo "  ✓ test email found via search API"

# 5. clean shutdown
echo "== 5. shutdown =="
kill "$MAILPIT_PID"
wait "$MAILPIT_PID" 2>/dev/null || true
MAILPIT_PID=""
echo "  ✓ clean shutdown"

echo ""
echo "== ALL CHECKS PASSED for $MAILPIT_VER =="
