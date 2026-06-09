use std::collections::BTreeMap;
use std::io;
#[cfg(test)]
use std::time::Duration;

use state::{EnvContextValues, PvPaths, ResourceAllocationRecord};

use crate::managed_resources::sql::{
    self, SqlAdminContext, SqlAllocationRequest, SqlEngine, sql_resource_env,
};
use crate::managed_resources::{
    ManagedResourcePortSpec, ManagedResourceReadiness, ManagedResourceRuntimeAdapter,
    ManagedResourceRuntimeContext, RESOURCE_HOST,
};
use crate::{DaemonError, ProcessSpec};

const POSTGRES_ADMIN_USERNAME: &str = "pv_root";
const POSTGRES_PORTS: &[ManagedResourcePortSpec] = &[ManagedResourcePortSpec {
    name: "postgres",
    preferred_port: 5432,
}];

#[derive(Clone, Debug)]
pub(crate) struct PostgresRuntimeAdapter {
    #[cfg(test)]
    readiness_timeout: Duration,
}

impl PostgresRuntimeAdapter {
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(test)]
            readiness_timeout: super::RESOURCE_READINESS_TIMEOUT,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_readiness_timeout(readiness_timeout: Duration) -> Self {
        Self {
            readiness_timeout,
            ..Self::new()
        }
    }
}

impl ManagedResourceRuntimeAdapter for PostgresRuntimeAdapter {
    fn resource_name(&self) -> &'static str {
        "postgres"
    }

    fn artifact_adapter(&self) -> Result<resources::RuntimeArtifactAdapter, DaemonError> {
        resources::postgres_adapter().map_err(Into::into)
    }

    fn port_specs(&self) -> &'static [ManagedResourcePortSpec] {
        POSTGRES_PORTS
    }

    fn build_process_spec(
        &self,
        paths: &PvPaths,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ProcessSpec, DaemonError> {
        let admin = admin_context(context)?;
        initialize_data_dir_if_missing(paths, context, &admin)?;
        write_postgres_config(paths, context, admin.port)?;

        Ok(ProcessSpec {
            name: format!("{}-{}", context.resource_name, context.track),
            command: self
                .artifact_adapter()?
                .executable_path(&context.artifact_path),
            arguments: vec![
                "-D".to_string(),
                context.data_dir.to_string(),
                "-h".to_string(),
                RESOURCE_HOST.to_string(),
                "-p".to_string(),
                admin.port.to_string(),
            ],
            private_environment: BTreeMap::new(),
            config_path: paths.resource_runtime_config(&context.resource_name, &context.track),
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
        let admin = admin_context(context)?;

        Ok(ManagedResourceReadiness::async_check(
            "sql:postgres:admin",
            move || {
                let admin = admin.clone();

                Box::pin(async move { sql::ping_admin(&admin, SqlEngine::Postgres).await })
            },
        ))
    }

    #[cfg(test)]
    fn readiness_timeout(&self) -> Duration {
        self.readiness_timeout
    }

    fn resource_env(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<EnvContextValues, DaemonError> {
        Ok(sql_resource_env(
            &admin_context(context)?,
            SqlEngine::Postgres,
        ))
    }

    fn reconcile_allocations<'a>(
        &'a self,
        _paths: &'a PvPaths,
        database: &'a mut state::Database,
        context: &'a ManagedResourceRuntimeContext,
        _resource_env: &'a EnvContextValues,
        allocations: &'a [ResourceAllocationRecord],
    ) -> crate::managed_resources::ManagedResourceAllocationFuture<'a> {
        Box::pin(async move {
            let admin = admin_context(context)?;

            for allocation in allocations {
                sql::ensure_database_allocation(
                    database,
                    SqlAllocationRequest {
                        project_id: &allocation.project_id,
                        resource_name: &allocation.resource_name,
                        track: &allocation.track,
                        allocation_name: &allocation.allocation_name,
                        engine: SqlEngine::Postgres,
                        context: &admin,
                    },
                )
                .await?;
            }

            Ok(())
        })
    }
}

fn initialize_data_dir_if_missing(
    paths: &PvPaths,
    context: &ManagedResourceRuntimeContext,
    admin: &SqlAdminContext,
) -> Result<(), DaemonError> {
    if data_dir_is_initialized(context)? {
        return Ok(());
    }

    let password_file = postgres_initdb_password_file(paths, context);
    state::fs::write_sensitive_file(&password_file, &admin.password)?;
    let init_result = run_initdb(context, admin, &password_file);
    let cleanup_result = delete_optional_file(&password_file);

    match (init_result, cleanup_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn postgres_initdb_password_file(
    paths: &PvPaths,
    context: &ManagedResourceRuntimeContext,
) -> camino::Utf8PathBuf {
    paths.run().join(format!(
        "resources/{}-{}.initdb-password",
        context.resource_name, context.track
    ))
}

fn data_dir_is_initialized(context: &ManagedResourceRuntimeContext) -> Result<bool, DaemonError> {
    match state::fs::read_to_string(&context.data_dir.join("PG_VERSION")) {
        Ok(_) => Ok(true),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "Postgres adapter owns the one-shot initdb process required before supervised startup"
)]
fn run_initdb(
    context: &ManagedResourceRuntimeContext,
    admin: &SqlAdminContext,
    password_file: &camino::Utf8Path,
) -> Result<(), DaemonError> {
    let initdb = context.artifact_path.join("bin/initdb");
    let output = std::process::Command::new(initdb.as_std_path())
        .arg("-D")
        .arg(context.data_dir.as_str())
        .arg("--username")
        .arg(&admin.username)
        .arg("--pwfile")
        .arg(password_file.as_str())
        .arg("--auth-host")
        .arg("scram-sha-256")
        .arg("--auth-local")
        .arg("trust")
        .output()?;

    if output.status.success() {
        return Ok(());
    }

    Err(DaemonError::UnexpectedProtocolResponse {
        reason: format!(
            "{} initdb failed: {}",
            context.resource_name,
            String::from_utf8_lossy(&output.stderr)
        ),
    })
}

fn write_postgres_config(
    paths: &PvPaths,
    context: &ManagedResourceRuntimeContext,
    port: u16,
) -> Result<(), DaemonError> {
    let config = format!(
        "listen_addresses = '{RESOURCE_HOST}'\nport = {port}\nunix_socket_directories = ''\n"
    );

    state::fs::write_sensitive_file(&context.data_dir.join("postgresql.conf"), &config)?;
    state::fs::write_sensitive_file(
        &paths.resource_runtime_config(&context.resource_name, &context.track),
        &config,
    )?;

    Ok(())
}

fn admin_context(context: &ManagedResourceRuntimeContext) -> Result<SqlAdminContext, DaemonError> {
    Ok(SqlAdminContext {
        host: RESOURCE_HOST.to_string(),
        port: required_port(context)?,
        username: POSTGRES_ADMIN_USERNAME.to_string(),
        password: context
            .env
            .get("password")
            .cloned()
            .filter(|password| !password.is_empty())
            .map_or_else(generate_hex_password, Ok)?,
    })
}

fn required_port(context: &ManagedResourceRuntimeContext) -> Result<u16, DaemonError> {
    context
        .ports
        .get("postgres")
        .copied()
        .ok_or_else(|| DaemonError::ManagedResourcePortMissing {
            resource: context.resource_name.clone(),
            track: context.track.clone(),
            port: "postgres".to_string(),
        })
}

fn generate_hex_password() -> Result<String, DaemonError> {
    let mut bytes = [0_u8; 16];
    fill_random_bytes(&mut bytes)?;

    Ok(hex_string(&bytes))
}

#[expect(
    clippy::disallowed_types,
    reason = "Postgres adapter generates local credentials from the OS random device"
)]
fn fill_random_bytes(bytes: &mut [u8]) -> Result<(), DaemonError> {
    let mut file = std::fs::File::open("/dev/urandom")?;
    std::io::Read::read_exact(&mut file, bytes)?;

    Ok(())
}

fn hex_string(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }

    output
}

fn delete_optional_file(path: &camino::Utf8Path) -> Result<(), DaemonError> {
    match state::fs::delete_file(path) {
        Ok(()) => Ok(()),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
