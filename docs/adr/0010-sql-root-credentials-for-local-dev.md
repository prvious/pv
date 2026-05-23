# Use shared SQL root credentials for local dev

PV will expose one PV-managed root/superuser credential per MySQL or Postgres Managed Resource instance/track instead of creating per-Project or per-allocation users. This favors low-friction local development and easier database inspection in tools like TablePlus, at the cost of weaker local isolation between Projects.
