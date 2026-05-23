# Use `pf` redirects for loopback ports 80 and 443

PV will use macOS `pf` redirect rules with a PV-owned anchor to redirect loopback ports `80` and `443` to unprivileged Gateway high ports. This preserves normal `https://project.test` URLs without running the PV daemon or Gateway as root, and is simpler for v1 than a privileged helper or launchd socket activation. PV only manages its own anchor and anchor reference so existing non-PV `pf` rules are preserved.
