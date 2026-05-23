# Create Resource allocations inside shared Managed Resources

PV will not run isolated per-Project service instances, but Project config may ask PV to create Resource allocations inside shared machine-level Managed Resource instances. This keeps runtime orchestration manageable while still giving Projects separate databases, buckets, users, credentials, or equivalent resource-specific objects where the Managed Resource supports them.
