use std::collections::BTreeMap;

use camino::Utf8Path;
use state::{
    Database, EnvContextValues, PvPaths, ResourceAllocationRecord, ResourceAllocationStatus,
};

use crate::managed_resources::{
    ManagedResourceArtifactAdapter, ManagedResourcePortSpec, ManagedResourceReadiness,
    ManagedResourceRuntimeAdapter, ManagedResourceRuntimeContext, RESOURCE_HOST,
};
use crate::{DaemonError, ProcessSpec, ReadinessCheck};

const PORTS: &[ManagedResourcePortSpec] = &[ManagedResourcePortSpec {
    name: "redis",
    preferred_port: 6379,
}];

#[derive(Clone, Debug, Default)]
pub(crate) struct RedisRuntimeAdapter;

impl RedisRuntimeAdapter {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl ManagedResourceRuntimeAdapter for RedisRuntimeAdapter {
    fn resource_name(&self) -> &'static str {
        "redis"
    }

    fn artifact_adapter(&self) -> Result<ManagedResourceArtifactAdapter, DaemonError> {
        ManagedResourceArtifactAdapter::new("redis", "bin/redis-server")
    }

    fn port_specs(&self) -> &'static [ManagedResourcePortSpec] {
        PORTS
    }

    fn build_process_spec(
        &self,
        paths: &PvPaths,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ProcessSpec, DaemonError> {
        let port = required_port(context, "redis")?;
        let config_path = paths.resource_runtime_config(&context.resource_name, &context.track);
        let arguments = vec![
            "--bind".to_string(),
            RESOURCE_HOST.to_string(),
            "--port".to_string(),
            port.to_string(),
            "--dir".to_string(),
            context.data_dir.as_str().to_string(),
            "--save".to_string(),
            String::new(),
            "--appendonly".to_string(),
            "no".to_string(),
        ];
        let config = serde_json::json!({
            "resource": context.resource_name,
            "track": context.track,
            "port": port,
            "data_dir": context.data_dir.as_str(),
            "arguments": arguments.clone(),
        });

        create_dir_all(&context.data_dir)?;
        state::fs::write_sensitive_file(&config_path, &serde_json::to_string_pretty(&config)?)?;

        Ok(ProcessSpec {
            name: format!("{}-{}", context.resource_name, context.track),
            command: self
                .artifact_adapter()?
                .executable_path(&context.artifact_path),
            arguments,
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
        Ok(ReadinessCheck::RedisPing {
            host: RESOURCE_HOST.to_string(),
            port: required_port(context, "redis")?,
        }
        .into())
    }

    fn resource_env(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<EnvContextValues, DaemonError> {
        let port = required_port(context, "redis")?;

        Ok(redis_env(port, None))
    }

    fn reconcile_allocations<'a>(
        &'a self,
        _paths: &'a PvPaths,
        database: &'a mut Database,
        context: &'a ManagedResourceRuntimeContext,
        allocations: &'a [ResourceAllocationRecord],
    ) -> super::ManagedResourceAllocationFuture<'a> {
        Box::pin(async move {
            let port = required_port(context, "redis")?;

            for allocation in allocations {
                let env = redis_env(port, Some(&allocation.generated_name));
                if allocation.status != ResourceAllocationStatus::Ready {
                    database.mark_resource_allocation_ready(
                        &allocation.project_id,
                        &allocation.resource_name,
                        &allocation.track,
                        &allocation.allocation_name,
                        &env,
                    )?;
                }
            }

            Ok(())
        })
    }
}

fn redis_env(port: u16, prefix: Option<&str>) -> EnvContextValues {
    let mut env = BTreeMap::from([
        ("host".to_string(), RESOURCE_HOST.to_string()),
        ("port".to_string(), port.to_string()),
        (
            "url".to_string(),
            format!("redis://{RESOURCE_HOST}:{port}/0"),
        ),
    ]);

    if let Some(prefix) = prefix {
        env.insert("prefix".to_string(), prefix.to_string());
    }

    env
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

#[expect(
    clippy::disallowed_methods,
    reason = "Redis runtime adapter creates its version-scoped data directory before starting redis-server"
)]
fn create_dir_all(path: &Utf8Path) -> Result<(), DaemonError> {
    std::fs::create_dir_all(path)?;

    Ok(())
}
