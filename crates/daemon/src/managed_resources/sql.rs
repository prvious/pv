#![expect(
    dead_code,
    reason = "MySQL and Postgres adapter PRs consume the shared SQL foundation"
)]

use sqlx::mysql::{MySqlConnectOptions, MySqlPool};
use sqlx::postgres::{PgConnectOptions, PgPool, PgSslMode};
use state::{
    Database, EnvContextValues, ResourceAllocationRecord, ResourceAllocationStatus, StateError,
};

use crate::DaemonError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SqlEngine {
    Mysql,
    Postgres,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SqlAdminContext {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SqlAllocationContext {
    pub database: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SqlAllocationRequest<'a> {
    pub project_id: &'a str,
    pub resource_name: &'a str,
    pub track: &'a str,
    pub allocation_name: &'a str,
    pub engine: SqlEngine,
    pub context: &'a SqlAdminContext,
}

pub(crate) fn sql_resource_env(context: &SqlAdminContext, engine: SqlEngine) -> EnvContextValues {
    env_context(&[
        ("host", context.host.as_str()),
        ("password", context.password.as_str()),
        ("port", &context.port.to_string()),
        ("url", &sql_resource_url(context, engine)),
        ("username", context.username.as_str()),
    ])
}

pub(crate) fn sql_allocation_env(
    context: &SqlAllocationContext,
    engine: SqlEngine,
) -> EnvContextValues {
    env_context(&[
        ("database", context.database.as_str()),
        ("host", context.host.as_str()),
        ("password", context.password.as_str()),
        ("port", &context.port.to_string()),
        ("url", &sql_allocation_url(context, engine)),
        ("username", context.username.as_str()),
    ])
}

pub(crate) async fn ping_admin(
    context: &SqlAdminContext,
    engine: SqlEngine,
) -> Result<(), DaemonError> {
    match engine {
        SqlEngine::Mysql => {
            let pool = MySqlPool::connect_with(mysql_options(context)).await?;
            sqlx::query("SELECT 1").execute(&pool).await?;
        }
        SqlEngine::Postgres => {
            let pool = PgPool::connect_with(postgres_options(context)).await?;
            sqlx::query("SELECT 1").execute(&pool).await?;
        }
    }

    Ok(())
}

pub(crate) async fn create_database_if_missing(
    context: &SqlAdminContext,
    engine: SqlEngine,
    database: &str,
) -> Result<(), DaemonError> {
    let statement = create_database_statement(engine, database)?;

    match engine {
        SqlEngine::Mysql => {
            let pool = MySqlPool::connect_with(mysql_options(context)).await?;

            sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&pool)
                .await?;
        }
        SqlEngine::Postgres => {
            let pool = PgPool::connect_with(postgres_options(context)).await?;
            let exists = sqlx::query("SELECT 1 FROM pg_database WHERE datname = $1")
                .bind(database)
                .fetch_optional(&pool)
                .await?;

            if exists.is_none() {
                sqlx::query(sqlx::AssertSqlSafe(statement))
                    .execute(&pool)
                    .await?;
            }
        }
    }

    Ok(())
}

pub(crate) async fn ensure_database_allocation(
    database: &mut Database,
    request: SqlAllocationRequest<'_>,
) -> Result<(), DaemonError> {
    let allocation = desired_database_allocation(database, request)?;

    create_database_if_missing(request.context, request.engine, &allocation.generated_name).await?;
    mark_database_allocation_ready(database, request, allocation)
}

#[cfg(test)]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RecordingSqlAdmin {
    operations: Vec<RecordedSqlAdminOperation>,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordedSqlAdminOperation {
    pub engine: SqlEngine,
    pub database: String,
    pub statement: String,
}

#[cfg(test)]
impl RecordingSqlAdmin {
    pub(crate) fn operations(&self) -> &[RecordedSqlAdminOperation] {
        &self.operations
    }

    async fn create_database_if_missing(
        &mut self,
        engine: SqlEngine,
        database: &str,
    ) -> Result<(), DaemonError> {
        self.operations.push(RecordedSqlAdminOperation {
            engine,
            database: database.to_string(),
            statement: create_database_statement(engine, database)?,
        });

        Ok(())
    }
}

#[cfg(test)]
pub(crate) async fn ensure_database_allocation_for_test(
    database: &mut Database,
    admin: &mut RecordingSqlAdmin,
    request: SqlAllocationRequest<'_>,
) -> Result<(), DaemonError> {
    let allocation = desired_database_allocation(database, request)?;

    admin
        .create_database_if_missing(request.engine, &allocation.generated_name)
        .await?;

    mark_database_allocation_ready(database, request, allocation)
}

fn desired_database_allocation(
    database: &Database,
    request: SqlAllocationRequest<'_>,
) -> Result<ResourceAllocationRecord, DaemonError> {
    let allocation = database
        .resource_allocations(request.project_id, request.resource_name)?
        .into_iter()
        .find(|allocation| {
            allocation.track == request.track
                && allocation.allocation_name == request.allocation_name
        })
        .ok_or_else(|| StateError::ResourceAllocationNotFound {
            project_id: request.project_id.to_string(),
            resource: request.resource_name.to_string(),
            allocation: request.allocation_name.to_string(),
        })?;

    Ok(allocation)
}

fn mark_database_allocation_ready(
    database: &mut Database,
    request: SqlAllocationRequest<'_>,
    allocation: ResourceAllocationRecord,
) -> Result<(), DaemonError> {
    let allocation_context = SqlAllocationContext {
        database: allocation.generated_name,
        host: request.context.host.clone(),
        port: request.context.port,
        username: request.context.username.clone(),
        password: request.context.password.clone(),
    };
    let env = sql_allocation_env(&allocation_context, request.engine);

    match allocation.status {
        ResourceAllocationStatus::Desired => {
            database.mark_resource_allocation_ready(
                request.project_id,
                request.resource_name,
                request.track,
                request.allocation_name,
                &env,
            )?;
        }
        ResourceAllocationStatus::Ready if allocation.env != env => {
            database.record_resource_allocation_env_context(
                request.project_id,
                request.resource_name,
                request.track,
                request.allocation_name,
                &env,
            )?;
        }
        _ => {}
    }

    Ok(())
}

fn mysql_options(context: &SqlAdminContext) -> MySqlConnectOptions {
    MySqlConnectOptions::new()
        .host(&context.host)
        .port(context.port)
        .username(&context.username)
        .password(&context.password)
}

pub(crate) fn postgres_options(context: &SqlAdminContext) -> PgConnectOptions {
    PgConnectOptions::new()
        .host(&context.host)
        .port(context.port)
        .username(&context.username)
        .password(&context.password)
        .database("postgres")
        .ssl_mode(PgSslMode::Disable)
        .application_name("pv")
}

fn sql_resource_url(context: &SqlAdminContext, engine: SqlEngine) -> String {
    let username = percent_encode_userinfo(&context.username);
    let password = percent_encode_userinfo(&context.password);

    format!(
        "{}://{}:{}@{}:{}",
        engine.url_scheme(),
        username,
        password,
        context.host,
        context.port
    )
}

fn sql_allocation_url(context: &SqlAllocationContext, engine: SqlEngine) -> String {
    let username = percent_encode_userinfo(&context.username);
    let password = percent_encode_userinfo(&context.password);

    format!(
        "{}://{}:{}@{}:{}/{}",
        engine.url_scheme(),
        username,
        password,
        context.host,
        context.port,
        context.database
    )
}

fn create_database_statement(engine: SqlEngine, database: &str) -> Result<String, DaemonError> {
    let identifier = quote_database_identifier(engine, database)?;
    let statement = match engine {
        SqlEngine::Mysql => format!("CREATE DATABASE IF NOT EXISTS {identifier}"),
        SqlEngine::Postgres => format!("CREATE DATABASE {identifier}"),
    };

    Ok(statement)
}

fn quote_database_identifier(engine: SqlEngine, identifier: &str) -> Result<String, DaemonError> {
    let quote = match engine {
        SqlEngine::Mysql => '`',
        SqlEngine::Postgres => '"',
    };

    quote_ascii_identifier(identifier, quote)
}

fn quote_ascii_identifier(identifier: &str, quote: char) -> Result<String, DaemonError> {
    if identifier.is_empty()
        || !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(DaemonError::InvalidSqlIdentifier {
            identifier: identifier.to_string(),
        });
    }

    let mut quoted = String::with_capacity(identifier.len() + 2);
    quoted.push(quote);
    quoted.push_str(identifier);
    quoted.push(quote);

    Ok(quoted)
}

fn env_context(entries: &[(&str, &str)]) -> EnvContextValues {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

fn percent_encode_userinfo(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());

    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            push_percent_encoded_byte(&mut encoded, byte);
        }
    }

    encoded
}

fn push_percent_encoded_byte(encoded: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    encoded.push('%');
    encoded.push(char::from(HEX[usize::from(byte >> 4)]));
    encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
}

impl SqlEngine {
    const fn url_scheme(self) -> &'static str {
        match self {
            Self::Mysql => "mysql",
            Self::Postgres => "postgres",
        }
    }
}
