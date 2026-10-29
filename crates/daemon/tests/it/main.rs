//! The daemon's integration tests, in one binary so they compile and link once.
//! `platform_preflight` stays a separate binary because Linux and Windows CI build it alone.

// `sql_foundation` includes the daemon's own `sql.rs`, which names `crate::DaemonError`.
use daemon::DaemonError;

mod caddy_admin;
mod daemon_foundation;
mod gateway_config;
mod gateway_reconciliation;
mod gateway_runtime_contracts;
mod loopback_port_probe;
mod monitor;
mod project_env_reconciliation;
mod project_hooks;
mod real_artifact_gateway_e2e;
mod real_artifact_resource_matrix;
mod sql_foundation;
mod supervisor_foundation;
