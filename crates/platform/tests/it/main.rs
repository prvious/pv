//! The platform crate's integration tests, in one binary so they compile and link once.
//! The `unsupported_*` tests stay separate binaries because Linux and Windows CI build them alone.

mod boot_session;
mod hooks;
mod launch_agent;
mod linux_daemon_registration;
mod linux_helper;
mod process_identity;
mod resolver_config;
