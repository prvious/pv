# Use an internal DNS resolver for `.test`

PV needs wildcard `.test` hostname resolution on macOS. We will use an internal lightweight DNS resolver managed by PV instead of per-Project `/etc/hosts` entries, `dnsmasq`, or CoreDNS because PV only needs a narrow `.test` resolver, should avoid external service/config drift, and should report resolver health directly in `pv status`. macOS will be configured through `/etc/resolver/test`, created by `pv setup`, so the resolver can run as part of PV while `.test` lookups remain scoped away from global DNS settings.
