use std::collections::BTreeMap;
use std::io::{self, Read};
#[cfg(test)]
use std::net::TcpListener;

use camino::{Utf8Path, Utf8PathBuf};
use state::{Database, EnvContextValues, PvPaths, ResourceAllocationRecord};

#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use tokio::net::TcpStream;
#[cfg(test)]
use tokio::sync::Mutex;

use super::{
    ManagedResourceAllocationFuture, ManagedResourceArtifactAdapter, ManagedResourcePortSpec,
    ManagedResourcePreparationFuture, ManagedResourceReadiness, ManagedResourceReadinessFuture,
    ManagedResourceRuntimeAdapter, ManagedResourceRuntimeContext, RESOURCE_HOST, sql,
};
#[cfg(test)]
use super::{
    ManagedResourceInstallOptions, ManagedResourceRuntimeCatalog, current_target_platform,
};
use crate::{DaemonError, ProcessSpec};

pub(crate) const RESOURCE_NAME: &str = "mysql";
const MYSQL_PORT_NAME: &str = "mysql";
const MYSQL_USERNAME: &str = "pv_root";
const PASSWORD_HEX_BYTES: usize = 16;
const HEX: &[u8; 16] = b"0123456789abcdef";

const PORTS: &[ManagedResourcePortSpec] = &[ManagedResourcePortSpec {
    name: MYSQL_PORT_NAME,
    preferred_port: 3306,
}];

#[derive(Clone, Debug)]
pub(crate) struct MysqlRuntimeAdapter {
    admin: MysqlAdmin,
    ports: &'static [ManagedResourcePortSpec],
}

#[derive(Clone, Debug)]
enum MysqlAdmin {
    Production,
    #[cfg(test)]
    Recording(RecordingMysqlAdmin),
}

#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(crate) struct RecordingMysqlAdmin {
    inner: Arc<Mutex<sql::RecordingSqlAdmin>>,
}

impl MysqlRuntimeAdapter {
    pub(crate) fn new() -> Self {
        Self {
            admin: MysqlAdmin::Production,
            ports: PORTS,
        }
    }

    #[cfg(test)]
    fn with_recording_admin(admin: RecordingMysqlAdmin) -> Result<Self, DaemonError> {
        let preferred_port = available_test_port()?;
        let ports = Box::leak(Box::new([ManagedResourcePortSpec {
            name: MYSQL_PORT_NAME,
            preferred_port,
        }]));

        Ok(Self {
            admin: MysqlAdmin::Recording(admin),
            ports,
        })
    }

    fn admin_context(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<sql::SqlAdminContext, DaemonError> {
        let port = required_mysql_port(context)?;
        let username = context
            .env
            .get("username")
            .cloned()
            .unwrap_or_else(|| MYSQL_USERNAME.to_string());
        let password = match context.env.get("password") {
            Some(password) => password.clone(),
            None => generated_password()?,
        };

        Ok(sql::SqlAdminContext {
            host: RESOURCE_HOST.to_string(),
            port,
            username,
            password,
        })
    }
}

impl ManagedResourceRuntimeAdapter for MysqlRuntimeAdapter {
    fn resource_name(&self) -> &'static str {
        RESOURCE_NAME
    }

    fn artifact_adapter(&self) -> Result<ManagedResourceArtifactAdapter, DaemonError> {
        ManagedResourceArtifactAdapter::new(RESOURCE_NAME, "bin/mysqld")
    }

    fn port_specs(&self) -> &'static [ManagedResourcePortSpec] {
        self.ports
    }

    fn prepare_runtime<'a>(
        &'a self,
        _paths: &'a PvPaths,
        context: &'a ManagedResourceRuntimeContext,
    ) -> ManagedResourcePreparationFuture<'a> {
        Box::pin(async move {
            let executable = self
                .artifact_adapter()?
                .executable_path(&context.artifact_path);
            let data_dir = context.data_dir.clone();
            let artifact_path = context.artifact_path.clone();

            tokio::task::spawn_blocking(move || {
                initialize_data_dir_if_missing(&executable, &data_dir, &artifact_path)
            })
            .await??;

            Ok(())
        })
    }

    fn build_process_spec(
        &self,
        paths: &PvPaths,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ProcessSpec, DaemonError> {
        let adapter = self.artifact_adapter()?;
        let mysql_port = required_mysql_port(context)?;
        let socket_path = mysql_socket_path(paths, &context.track);
        let init_file_path = mysql_init_file_path(paths, &context.track);
        let config_path = paths.resource_runtime_config(&context.resource_name, &context.track);
        let config = serde_json::json!({
            "resource": context.resource_name,
            "track": context.track,
            "data_dir": context.data_dir.as_str(),
            "mysql_port": mysql_port,
            "socket": socket_path.as_str(),
            "init_file": init_file_path.as_str(),
        });

        ensure_parent_dir(&socket_path)?;
        write_mysql_init_file(&init_file_path, &self.admin_context(context)?)?;
        state::fs::write_sensitive_file(&config_path, &serde_json::to_string_pretty(&config)?)?;

        Ok(ProcessSpec {
            name: format!("{}-{}", context.resource_name, context.track),
            command: adapter.executable_path(&context.artifact_path),
            arguments: vec![
                "--no-defaults".to_string(),
                "--datadir".to_string(),
                context.data_dir.to_string(),
                "--bind-address=127.0.0.1".to_string(),
                "--port".to_string(),
                mysql_port.to_string(),
                "--socket".to_string(),
                socket_path.to_string(),
                "--init-file".to_string(),
                init_file_path.to_string(),
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
        let admin = self.admin.clone();
        let admin_context = self.admin_context(context)?;

        Ok(ManagedResourceReadiness::async_check(
            "mysql-admin",
            move || {
                let admin = admin.clone();
                let admin_context = admin_context.clone();

                Box::pin(async move { admin.ping_admin(&admin_context).await })
                    as ManagedResourceReadinessFuture<'static>
            },
        ))
    }

    fn resource_env(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<EnvContextValues, DaemonError> {
        let admin_context = self.admin_context(context)?;

        Ok(sql::sql_resource_env(&admin_context, sql::SqlEngine::Mysql))
    }

    fn reconcile_allocations<'a>(
        &'a self,
        _paths: &'a PvPaths,
        database: &'a mut Database,
        context: &'a ManagedResourceRuntimeContext,
        _resource_env: &'a EnvContextValues,
        allocations: &'a [ResourceAllocationRecord],
    ) -> ManagedResourceAllocationFuture<'a> {
        Box::pin(async move {
            let admin_context = self.admin_context(context)?;

            for allocation in allocations {
                self.admin
                    .ensure_database_allocation(database, allocation, &admin_context)
                    .await?;
            }

            Ok(())
        })
    }
}

impl MysqlAdmin {
    async fn ping_admin(&self, context: &sql::SqlAdminContext) -> Result<(), DaemonError> {
        match self {
            Self::Production => sql::ping_admin(context, sql::SqlEngine::Mysql).await,
            #[cfg(test)]
            Self::Recording(admin) => admin.ping_admin(context).await,
        }
    }

    async fn ensure_database_allocation(
        &self,
        database: &mut Database,
        allocation: &ResourceAllocationRecord,
        context: &sql::SqlAdminContext,
    ) -> Result<(), DaemonError> {
        let request = sql::SqlAllocationRequest {
            project_id: &allocation.project_id,
            resource_name: RESOURCE_NAME,
            track: &allocation.track,
            allocation_name: &allocation.allocation_name,
            engine: sql::SqlEngine::Mysql,
            context,
        };

        match self {
            Self::Production => sql::ensure_database_allocation(database, request).await,
            #[cfg(test)]
            Self::Recording(admin) => admin.ensure_database_allocation(database, request).await,
        }
    }
}

#[cfg(test)]
impl RecordingMysqlAdmin {
    pub(crate) async fn operations(
        &self,
    ) -> Result<Vec<sql::RecordedSqlAdminOperation>, DaemonError> {
        let admin = self.inner.lock().await;

        Ok(admin.operations().to_vec())
    }

    async fn ping_admin(&self, context: &sql::SqlAdminContext) -> Result<(), DaemonError> {
        let _stream = TcpStream::connect((context.host.as_str(), context.port)).await?;

        Ok(())
    }

    async fn ensure_database_allocation(
        &self,
        database: &mut Database,
        request: sql::SqlAllocationRequest<'_>,
    ) -> Result<(), DaemonError> {
        let mut admin = self.inner.lock().await;

        sql::ensure_database_allocation_for_test(database, &mut admin, request).await
    }
}

#[cfg(test)]
pub(crate) fn mysql_runtime_catalog_with_recording_admin(
    manifest_url: &str,
    admin: RecordingMysqlAdmin,
) -> Result<ManagedResourceRuntimeCatalog, DaemonError> {
    Ok(ManagedResourceRuntimeCatalog::with_adapter(
        ManagedResourceInstallOptions {
            manifest_url: manifest_url.to_string(),
            target_platform: current_target_platform(),
        },
        MysqlRuntimeAdapter::with_recording_admin(admin)?,
    ))
}

#[cfg(test)]
fn available_test_port() -> Result<u16, DaemonError> {
    let listener = TcpListener::bind((RESOURCE_HOST, 0))?;
    let port = listener.local_addr()?.port();

    Ok(port)
}

fn required_mysql_port(context: &ManagedResourceRuntimeContext) -> Result<u16, DaemonError> {
    context.ports.get(MYSQL_PORT_NAME).copied().ok_or_else(|| {
        DaemonError::ManagedResourcePortMissing {
            resource: context.resource_name.clone(),
            track: context.track.clone(),
            port: MYSQL_PORT_NAME.to_string(),
        }
    })
}

fn mysql_socket_path(paths: &PvPaths, track: &str) -> Utf8PathBuf {
    paths.run().join(format!("resources/mysql-{track}.sock"))
}

fn mysql_init_file_path(paths: &PvPaths, track: &str) -> Utf8PathBuf {
    paths
        .run()
        .join(format!("resources/mysql-{track}.init.sql"))
}

fn write_mysql_init_file(
    path: &Utf8Path,
    context: &sql::SqlAdminContext,
) -> Result<(), DaemonError> {
    let username = mysql_string_literal(&context.username);
    let password = mysql_string_literal(&context.password);
    let host = mysql_string_literal(RESOURCE_HOST);
    let user = format!("{username}@{host}");
    let content = format!(
        "CREATE USER IF NOT EXISTS {user} IDENTIFIED BY {password};\n\
         ALTER USER {user} IDENTIFIED BY {password};\n\
         GRANT ALL PRIVILEGES ON *.* TO {user} WITH GRANT OPTION;\n\
         FLUSH PRIVILEGES;\n"
    );

    state::fs::write_sensitive_file(path, &content)?;

    Ok(())
}

fn mysql_string_literal(value: &str) -> String {
    let mut literal = String::with_capacity(value.len() + 2);

    literal.push('\'');
    for character in value.chars() {
        match character {
            '\'' | '\\' => {
                literal.push('\\');
                literal.push(character);
            }
            '\0' => literal.push_str("\\0"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\x1a' => literal.push_str("\\Z"),
            _ => literal.push(character),
        }
    }
    literal.push('\'');

    literal
}

fn initialize_data_dir_if_missing(
    executable: &Utf8Path,
    data_dir: &Utf8Path,
    artifact_path: &Utf8Path,
) -> Result<(), DaemonError> {
    if path_exists(&data_dir.join("mysql"))? {
        return Ok(());
    }

    create_dir_all(data_dir)?;
    run_initialize_insecure(executable, data_dir, artifact_path)
}

#[expect(
    clippy::disallowed_types,
    reason = "MySQL adapter owns running the managed mysqld initialization command"
)]
fn run_initialize_insecure(
    executable: &Utf8Path,
    data_dir: &Utf8Path,
    artifact_path: &Utf8Path,
) -> Result<(), DaemonError> {
    let status = std::process::Command::new(executable.as_std_path())
        .arg("--initialize-insecure")
        .arg("--datadir")
        .arg(data_dir.as_str())
        .arg("--basedir")
        .arg(artifact_path.as_str())
        .status()?;

    if status.success() {
        return Ok(());
    }

    Err(DaemonError::UnexpectedProtocolResponse {
        reason: format!("mysqld initialization failed with status {status}"),
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "MySQL adapter owns direct data directory creation before mysqld initialization"
)]
fn create_dir_all(path: &Utf8Path) -> Result<(), DaemonError> {
    std::fs::create_dir_all(path).map_err(Into::into)
}

fn ensure_parent_dir(path: &Utf8Path) -> Result<(), DaemonError> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "MySQL adapter checks whether the MySQL system database directory exists"
)]
fn path_exists(path: &Utf8Path) -> Result<bool, DaemonError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(source.into()),
    }
}

fn generated_password() -> Result<String, DaemonError> {
    let bytes = random_password_bytes()?;
    let mut password = String::with_capacity(PASSWORD_HEX_BYTES * 2);

    for byte in bytes {
        password.push(HEX[usize::from(byte >> 4)] as char);
        password.push(HEX[usize::from(byte & 0x0f)] as char);
    }

    Ok(password)
}

#[expect(
    clippy::disallowed_types,
    reason = "MySQL adapter generates local-only credentials from the OS random source"
)]
fn random_password_bytes() -> Result<[u8; PASSWORD_HEX_BYTES], DaemonError> {
    let mut bytes = [0_u8; PASSWORD_HEX_BYTES];
    let mut random = std::fs::File::open("/dev/urandom")?;
    random.read_exact(&mut bytes)?;

    Ok(bytes)
}
