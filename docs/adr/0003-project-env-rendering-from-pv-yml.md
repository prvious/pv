# Render Project `.env` values from `pv.yml` only inside a PV-owned block

PV will allow Projects to opt in to environment variable rendering from `pv.yml`, but the daemon may only write a clearly delimited PV-owned block inside the Project's `.env` file. This gives Laravel Projects ergonomic access to generated resource credentials and assigned ports while avoiding broad mutation of user-owned Project configuration.
