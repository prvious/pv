use std::collections::BTreeMap;
use std::time::Duration;

use state::{EnvContextValues, PvPaths};

use crate::managed_resources::{
    ManagedResourceArtifactAdapter, ManagedResourcePortSpec, ManagedResourceRuntimeAdapter,
    ManagedResourceRuntimeContext, RESOURCE_HOST,
};
use crate::{DaemonError, ProcessSpec, ReadinessCheck};

const FAKE_MAILPIT_PORTS: &[ManagedResourcePortSpec] = &[
    ManagedResourcePortSpec {
        name: "smtp",
        preferred_port: 1025,
        env_key: "smtp_port",
    },
    ManagedResourcePortSpec {
        name: "dashboard",
        preferred_port: 8025,
        env_key: "dashboard_port",
    },
];

#[derive(Clone, Debug)]
pub(crate) struct FakeMailpitRuntimeAdapter {
    artifact_adapter: ManagedResourceArtifactAdapter,
    readiness: FakeMailpitReadiness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FakeMailpitReadiness {
    Smtp,
    UnservedDashboardPort { timeout: Duration },
}

impl FakeMailpitRuntimeAdapter {
    pub(crate) fn new() -> Result<Self, DaemonError> {
        Ok(Self {
            artifact_adapter: ManagedResourceArtifactAdapter::new(
                "mailpit",
                "bin/pv-fake-mailpit",
            )?,
            readiness: FakeMailpitReadiness::Smtp,
        })
    }

    pub(crate) fn unready() -> Result<Self, DaemonError> {
        Ok(Self {
            artifact_adapter: ManagedResourceArtifactAdapter::new(
                "mailpit",
                "bin/pv-fake-mailpit",
            )?,
            readiness: FakeMailpitReadiness::UnservedDashboardPort {
                timeout: Duration::from_millis(100),
            },
        })
    }
}

impl ManagedResourceRuntimeAdapter for FakeMailpitRuntimeAdapter {
    fn resource_name(&self) -> &'static str {
        "mailpit"
    }

    fn artifact_adapter(&self) -> Result<ManagedResourceArtifactAdapter, DaemonError> {
        Ok(self.artifact_adapter.clone())
    }

    fn port_specs(&self) -> &'static [ManagedResourcePortSpec] {
        FAKE_MAILPIT_PORTS
    }

    fn build_process_spec(
        &self,
        paths: &PvPaths,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ProcessSpec, DaemonError> {
        let smtp_port = required_port(context, "smtp")?;
        let dashboard_port = required_port(context, "dashboard")?;
        let config_path = paths.resource_runtime_config(&context.resource_name, &context.track);
        let config = serde_json::json!({
            "resource": context.resource_name,
            "track": context.track,
            "smtp_port": smtp_port,
            "dashboard_port": dashboard_port,
            "data_dir": context.data_dir.as_str(),
        });

        state::fs::write_sensitive_file(&config_path, &serde_json::to_string_pretty(&config)?)?;

        Ok(ProcessSpec {
            name: format!("{}-{}", context.resource_name, context.track),
            command: self
                .artifact_adapter
                .executable_path(&context.artifact_path),
            arguments: vec![smtp_port.to_string(), dashboard_port.to_string()],
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
    ) -> Result<ReadinessCheck, DaemonError> {
        match self.readiness {
            FakeMailpitReadiness::Smtp => Ok(ReadinessCheck::Tcp {
                host: RESOURCE_HOST.to_string(),
                port: required_port(context, "smtp")?,
            }),
            FakeMailpitReadiness::UnservedDashboardPort { .. } => Ok(ReadinessCheck::Http {
                host: RESOURCE_HOST.to_string(),
                port: required_port(context, "dashboard")?,
                path: "/__pv_unready_fixture__".to_string(),
            }),
        }
    }

    fn readiness_timeout(&self) -> Duration {
        match self.readiness {
            FakeMailpitReadiness::Smtp => Duration::from_secs(15),
            FakeMailpitReadiness::UnservedDashboardPort { timeout } => timeout,
        }
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
