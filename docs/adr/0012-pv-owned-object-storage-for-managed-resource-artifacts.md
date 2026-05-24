# Use PV-owned object storage for Managed Resource artifacts

PV will publish Artifact manifests and normalized Managed Resource artifact archives through PV-owned object storage/CDN endpoints, such as Cloudflare R2 behind a PV-owned HTTPS domain, instead of making PV clients depend on GitHub Release asset URLs. This keeps artifact availability, URL stability, retention, and CDN behavior under PV's control while GitHub can remain useful for source tags, changelogs, and release automation.
