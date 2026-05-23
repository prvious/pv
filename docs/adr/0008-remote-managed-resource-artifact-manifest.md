# Use a remote manifest for Managed Resource artifacts

PV will discover Managed Resource artifacts from a PV-owned remote manifest instead of hardcoding versions in the app binary or scraping GitHub release asset names at runtime. The manifest records resource versions, resource-specific update tracks, platforms, URLs, checksums, sizes, defaults, schema version, and minimum supported PV version so artifact availability and update policy can change independently from PV application releases while compatibility failures remain explicit.
