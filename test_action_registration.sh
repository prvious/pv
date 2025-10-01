#!/bin/bash
# Simple test to verify dnsmasq action is registered

# Build the project
echo "Building project..."
go build -o bin/pv . || exit 1

# Run the CLI with timeout and capture output
echo "Running CLI to verify action registration..."
timeout 2 ./bin/pv 2>&1 > /tmp/cli_output.txt || true

# Check if the action name appears in the output
if grep -q "Setup DNSMasq for .local and .test domains" /tmp/cli_output.txt; then
    echo "✓ SUCCESS: DNSMasq action is properly registered and appears in the CLI"
    exit 0
else
    echo "✗ FAILED: DNSMasq action not found in CLI output"
    cat /tmp/cli_output.txt
    exit 1
fi
