# Separate PV app releases from Managed Resource artifact releases

PV application releases will be separate from PV-owned Managed Resource artifact releases. This lets binaries such as PHP, FrankenPHP, MySQL, PostgreSQL, Redis, Mailpit, and RustFS be rebuilt on their own cadence without forcing a new PV CLI/daemon release, while PV still controls artifact compatibility and metadata.
