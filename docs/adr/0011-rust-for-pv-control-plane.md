# Use Rust for the PV control plane

PV's CLI and daemon will be implemented in Rust. Rust gives PV a single-binary-friendly foundation with strong correctness around filesystem operations, state transitions, process supervision, socket protocols, and the internal DNS resolver, while Managed Resources remain external artifacts managed by the daemon.
