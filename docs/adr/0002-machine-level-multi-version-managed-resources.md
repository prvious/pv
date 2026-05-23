# Support machine-level multi-version Managed Resources

PV will run shared machine-level Managed Resource instances per resource/track instead of creating isolated per-Project service instances or forcing only one active track at a time. This accepts port-assignment complexity because track-scoped resource data keeps simultaneous tracks understandable, and the same port-conflict handling is needed whenever conventional ports are already occupied by non-PV processes.
