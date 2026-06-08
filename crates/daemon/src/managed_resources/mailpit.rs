use std::collections::BTreeMap;

use state::{EnvContextValues, PvPaths};

use crate::managed_resources::{
    ManagedResourceArtifactAdapter, ManagedResourcePortSpec, ManagedResourceReadiness,
    ManagedResourceRuntimeAdapter, ManagedResourceRuntimeContext, RESOURCE_HOST,
};
use crate::{DaemonError, ProcessSpec, ReadinessCheck};

const MAILPIT_PORTS: &[ManagedResourcePortSpec] = &[
    ManagedResourcePortSpec {
        name: "smtp",
        preferred_port: 1025,
    },
    ManagedResourcePortSpec {
        name: "dashboard",
        preferred_port: 8025,
    },
];

#[derive(Clone, Debug)]
pub(crate) struct MailpitRuntimeAdapter;

impl MailpitRuntimeAdapter {
    pub(crate) const NAME: &'static str = "mailpit";

    pub(crate) const fn new() -> Self {
        Self
    }
}

impl ManagedResourceRuntimeAdapter for MailpitRuntimeAdapter {
    fn resource_name(&self) -> &'static str {
        Self::NAME
    }

    fn artifact_adapter(&self) -> Result<ManagedResourceArtifactAdapter, DaemonError> {
        ManagedResourceArtifactAdapter::new(Self::NAME, "bin/mailpit")
    }

    fn port_specs(&self) -> &'static [ManagedResourcePortSpec] {
        MAILPIT_PORTS
    }

    fn build_process_spec(
        &self,
        paths: &PvPaths,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ProcessSpec, DaemonError> {
        let smtp_port = required_port(context, "smtp")?;
        let dashboard_port = required_port(context, "dashboard")?;
        let database_path = context.data_dir.join("mailpit.db");
        let config_path = paths.resource_runtime_config(&context.resource_name, &context.track);
        let config = serde_json::json!({
            "resource": context.resource_name,
            "track": context.track,
            "smtp": format!("{RESOURCE_HOST}:{smtp_port}"),
            "listen": format!("{RESOURCE_HOST}:{dashboard_port}"),
            "data_dir": context.data_dir.as_str(),
        });

        state::fs::ensure_user_dir(&context.data_dir)?;
        state::fs::write_sensitive_file(&config_path, &serde_json::to_string_pretty(&config)?)?;

        Ok(ProcessSpec {
            name: format!("{}-{}", context.resource_name, context.track),
            command: self
                .artifact_adapter()?
                .executable_path(&context.artifact_path),
            arguments: vec![
                "--smtp".to_string(),
                format!("{RESOURCE_HOST}:{smtp_port}"),
                "--listen".to_string(),
                format!("{RESOURCE_HOST}:{dashboard_port}"),
                "--database".to_string(),
                database_path.to_string(),
                "--disable-version-check".to_string(),
            ],
            private_environment: BTreeMap::new(),
            config_path,
            log_path: paths.resource_log(&context.resource_name, &context.track),
            pid_path: paths.resource_pid(&context.resource_name, &context.track),
            metadata_path: paths.resource_runtime_metadata(&context.resource_name, &context.track),
            resource_name: context.resource_name.clone(),
            track: context.track.clone(),
        })
    }

    fn readiness(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ManagedResourceReadiness, DaemonError> {
        Ok(ReadinessCheck::Http {
            host: RESOURCE_HOST.to_string(),
            port: required_port(context, "dashboard")?,
            path: "/".to_string(),
        }
        .into())
    }

    fn resource_env(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<EnvContextValues, DaemonError> {
        let smtp_port = required_port(context, "smtp")?;
        let dashboard_port = required_port(context, "dashboard")?;

        Ok(BTreeMap::from([
            ("smtp_host".to_string(), RESOURCE_HOST.to_string()),
            ("smtp_port".to_string(), smtp_port.to_string()),
            (
                "dashboard_url".to_string(),
                format!("http://{RESOURCE_HOST}:{dashboard_port}"),
            ),
        ]))
    }
}

fn required_port(
    context: &ManagedResourceRuntimeContext,
    port_name: &str,
) -> Result<u16, DaemonError> {
    context
        .ports
        .get(port_name)
        .copied()
        .ok_or_else(|| DaemonError::ManagedResourcePortMissing {
            resource: context.resource_name.clone(),
            track: context.track.clone(),
            port: port_name.to_string(),
        })
}
