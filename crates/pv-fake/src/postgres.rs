//! The `initdb` and `postgres` personas: the parts of PostgreSQL that PV's adapter and SQL client
//! use, following recordings of PostgreSQL 18.4 started by PV.
//!
//! `initdb` writes the persona's own files to the data directory: `PG_VERSION`, which PV checks,
//! the role in `initdb.username` and `initdb.password`, and one file per database under
//! `databases/`, so databases outlive a restart as real ones do.

use std::io::{self, Write};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use camino::{Utf8Path, Utf8PathBuf};
use futures_util::stream;
use pgwire::api::auth::sasl::SASLAuthStartupHandler;
use pgwire::api::auth::sasl::scram::{ScramAuth, gen_salted_password};
use pgwire::api::auth::{
    AuthSource, DefaultServerParameterProvider, LoginInfo, Password, StartupHandler,
};
use pgwire::api::portal::{Format, Portal};
use pgwire::api::query::ExtendedQueryHandler;
use pgwire::api::results::{
    DataRowEncoder, DescribePortalResponse, DescribeStatementResponse, FieldInfo, QueryResponse,
    Response, Tag,
};
use pgwire::api::stmt::{NoopQueryParser, StoredStatement};
use pgwire::api::{ClientInfo, ErrorHandler, PgWireServerHandlers, Type};
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use pgwire::tokio::process_socket;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::accept;

const PG_VERSION: &str = "18\n";
const SERVER_VERSION: &str = "18.4";
/// PostgreSQL's default `scram_iterations`.
const SCRAM_ITERATIONS: usize = 4096;
const SCRAM_SALT: &[u8] = b"pv-fake-postgres";
/// The databases `initdb` creates.
const INITIAL_DATABASES: [&str; 3] = ["postgres", "template0", "template1"];
const SELECT_ONE: &str = "SELECT 1";
const DATABASE_EXISTS: &str = "SELECT 1 FROM pg_database WHERE datname = $1";

/// Handles `initdb -D <dir> --username <name> --pwfile <file> --auth-host scram-sha-256
/// --auth-local <method>`. Returns its exit code.
pub(crate) fn initdb(argv: &[String]) -> Result<u8> {
    let (mut data_dir, mut username, mut password_file) = (None, None, None);
    let mut arguments = argv.iter().skip(1);
    while let Some(flag) = arguments.next() {
        let Some(value) = arguments.next() else {
            bail!("initdb {flag} needs a value");
        };
        match flag.as_str() {
            "-D" => data_dir = Some(Utf8PathBuf::from(value)),
            "--username" => username = Some(value),
            "--pwfile" => password_file = Some(Utf8PathBuf::from(value)),
            // The `postgres` persona only signs clients in with SCRAM, and serves no Unix socket.
            "--auth-host" if value == "scram-sha-256" => {}
            "--auth-local" => {}
            _ => bail!("unexpected initdb argument {flag} {value}"),
        }
    }
    let (Some(data_dir), Some(username), Some(password_file)) = (data_dir, username, password_file)
    else {
        bail!("expected `initdb -D <dir> --username <name> --pwfile <file>`");
    };
    if state::fs::path_is_directory(&data_dir)? && !state::fs::read_dir_paths(&data_dir)?.is_empty()
    {
        // initdb's message for a data directory that isn't empty.
        let _write_result = write!(
            io::stderr(),
            "initdb: error: directory \"{data_dir}\" exists but is not empty\ninitdb: hint: If \
             you want to create a new database system, either remove or empty the directory \
             \"{data_dir}\" or run initdb with an argument other than \"{data_dir}\".\n"
        );
        return Ok(1);
    }
    let password = state::fs::read_to_string(&password_file)?;
    // initdb reads the password from the file's first line.
    let password = password.lines().next().unwrap_or_default();

    state::fs::ensure_user_dir(&data_dir)?;
    state::fs::ensure_user_dir(&data_dir.join("databases"))?;
    for database in INITIAL_DATABASES {
        state::fs::write_sensitive_file(&database_path(&data_dir, database), "")?;
    }
    state::fs::write_sensitive_file(&data_dir.join("initdb.username"), username)?;
    state::fs::write_sensitive_file(&data_dir.join("initdb.password"), password)?;
    // Last: PV takes `PG_VERSION` to mean the data directory is initialized.
    state::fs::write_sensitive_file(&data_dir.join("PG_VERSION"), PG_VERSION)?;

    Ok(0)
}

/// Handles `postgres -D <dir> -h <host> -p <port>`, and serves until it is shut down.
pub(crate) async fn start(argv: &[String]) -> Result<Clients> {
    let (mut data_dir, mut host, mut port) = (None, None, None);
    let mut arguments = argv.iter().skip(1);
    while let Some(flag) = arguments.next() {
        let Some(value) = arguments.next() else {
            bail!("postgres {flag} needs a value");
        };
        match flag.as_str() {
            "-D" => data_dir = Some(Utf8PathBuf::from(value)),
            "-h" => host = Some(value.as_str()),
            "-p" => {
                port = Some(
                    value
                        .parse::<u16>()
                        .with_context(|| format!("port {value}"))?,
                )
            }
            _ => bail!("unexpected postgres argument {flag}"),
        }
    }
    let (Some(data_dir), Some(host), Some(port)) = (data_dir, host, port) else {
        bail!("expected `postgres -D <dir> -h <host> -p <port>`");
    };
    if !state::fs::path_is_file(&data_dir.join("PG_VERSION"))? {
        bail!("{data_dir} is not a database cluster; initdb hasn't run");
    }
    let password = state::fs::read_to_string(&data_dir.join("initdb.password"))?;
    let cluster = Arc::new(Cluster {
        username: state::fs::read_to_string(&data_dir.join("initdb.username"))?,
        salted_password: gen_salted_password(&password, SCRAM_SALT, SCRAM_ITERATIONS),
        data_dir,
        query_parser: Arc::new(NoopQueryParser::new()),
    });
    let listener = TcpListener::bind((host, port))
        .await
        .with_context(|| format!("binding PostgreSQL port {port}"))?;
    let (open, closed) = mpsc::channel(1);
    let accepting = tokio::spawn(async move {
        loop {
            let stream = accept(&listener).await;
            let (cluster, open) = (cluster.clone(), open.clone());
            tokio::spawn(async move {
                // Held until the client disconnects.
                let _open = open;
                let _result = process_socket(stream, None, Handlers { cluster }).await;
            });
        }
    });

    Ok(Clients { accepting, closed })
}

/// The connections a `postgres` persona serves.
pub(crate) struct Clients {
    accepting: JoinHandle<()>,
    closed: mpsc::Receiver<()>,
}

impl Clients {
    /// Stops accepting connections and waits until every open one has closed, as PostgreSQL's
    /// smart shutdown does. ponytail: PostgreSQL keeps its port open and refuses new clients
    /// with `57P03`; the persona closes it.
    pub(crate) async fn closed(mut self) {
        self.accepting.abort();
        // The accept loop holds a sender too, dropped with the aborted task.
        let _cancelled = self.accepting.await;
        // `None` once every connection's sender is dropped; nothing is ever sent.
        let _none = self.closed.recv().await;
    }
}

/// A data directory and the role `initdb` created in it.
#[derive(Debug)]
struct Cluster {
    data_dir: Utf8PathBuf,
    username: String,
    salted_password: Vec<u8>,
    query_parser: Arc<NoopQueryParser>,
}

impl Cluster {
    /// The file standing for a database, or `None` for a name PV never creates.
    fn database(&self, name: &str) -> Option<Utf8PathBuf> {
        let valid = !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');

        valid.then(|| database_path(&self.data_dir, name))
    }

    fn database_exists(&self, name: &str) -> bool {
        self.database(name)
            .is_some_and(|path| state::fs::path_exists(&path))
    }

    fn create_database(&self, name: &str) -> PgWireResult<Response> {
        let Some(path) = self.database(name) else {
            return Err(unsupported(&format!("CREATE DATABASE \"{name}\"")));
        };
        if state::fs::path_exists(&path) {
            return Err(error(
                "ERROR",
                "42P04",
                format!("database \"{name}\" already exists"),
            ));
        }
        state::fs::write_sensitive_file(&path, "")
            .map_err(|source| PgWireError::ApiError(Box::new(source)))?;

        Ok(Response::Execution(Tag::new("CREATE DATABASE")))
    }
}

fn database_path(data_dir: &Utf8Path, name: &str) -> Utf8PathBuf {
    data_dir.join("databases").join(name)
}

#[async_trait]
impl AuthSource for Cluster {
    async fn get_password(&self, login: &LoginInfo) -> PgWireResult<Password> {
        let user = login.user().unwrap_or_default();
        if user != self.username {
            // PostgreSQL refuses an unknown role as it does a wrong password.
            return Err(PgWireError::InvalidPassword(user.to_owned()));
        }

        Ok(Password::new(
            Some(SCRAM_SALT.to_vec()),
            self.salted_password.clone(),
        ))
    }
}

/// The statements PV's SQL client sends.
enum Statement {
    SelectOne,
    DatabaseExists,
    CreateDatabase(String),
}

impl Statement {
    fn parse(sql: &str) -> PgWireResult<Self> {
        if sql == SELECT_ONE {
            return Ok(Self::SelectOne);
        }
        if sql == DATABASE_EXISTS {
            return Ok(Self::DatabaseExists);
        }
        match sql
            .strip_prefix("CREATE DATABASE \"")
            .and_then(|rest| rest.strip_suffix('"'))
        {
            Some(name) => Ok(Self::CreateDatabase(name.to_owned())),
            None => Err(unsupported(sql)),
        }
    }

    fn parameter_types(&self) -> Vec<Type> {
        match self {
            Self::DatabaseExists => vec![Type::TEXT],
            Self::SelectOne | Self::CreateDatabase(_) => Vec::new(),
        }
    }

    /// Both queries return one unnamed `int4` column, as `SELECT 1` does.
    fn columns(&self, format: &Format) -> Vec<FieldInfo> {
        match self {
            Self::SelectOne | Self::DatabaseExists => vec![FieldInfo::new(
                "?column?".to_owned(),
                None,
                None,
                Type::INT4,
                format.format_for(0),
            )],
            Self::CreateDatabase(_) => Vec::new(),
        }
    }
}

fn ones(columns: Vec<FieldInfo>, count: usize) -> PgWireResult<Response> {
    let columns = Arc::new(columns);
    let rows = (0..count)
        .map(|_index| {
            let mut encoder = DataRowEncoder::new(columns.clone());
            encoder.encode_field(&1_i32)?;
            Ok(encoder.take_row())
        })
        .collect::<Vec<_>>();

    Ok(Response::Query(QueryResponse::new(
        columns,
        stream::iter(rows),
    )))
}

#[async_trait]
impl ExtendedQueryHandler for Cluster {
    type Statement = String;
    type QueryParser = NoopQueryParser;

    fn query_parser(&self) -> Arc<Self::QueryParser> {
        self.query_parser.clone()
    }

    async fn do_query<C>(
        &self,
        _client: &mut C,
        portal: &Portal<Self::Statement>,
        _max_rows: usize,
    ) -> PgWireResult<Response>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        let statement = Statement::parse(&portal.statement.statement)?;
        let columns = statement.columns(&portal.result_column_format);
        match statement {
            Statement::SelectOne => ones(columns, 1),
            Statement::DatabaseExists => {
                let name = portal.parameter::<String>(0, &Type::TEXT)?;
                let exists = name.is_some_and(|name| self.database_exists(&name));
                ones(columns, usize::from(exists))
            }
            Statement::CreateDatabase(name) => self.create_database(&name),
        }
    }

    async fn do_describe_statement<C>(
        &self,
        _client: &mut C,
        stored: &StoredStatement<Self::Statement>,
    ) -> PgWireResult<DescribeStatementResponse>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        let statement = Statement::parse(&stored.statement)?;

        Ok(DescribeStatementResponse::new(
            statement.parameter_types(),
            statement.columns(&Format::UnifiedText),
        ))
    }

    async fn do_describe_portal<C>(
        &self,
        _client: &mut C,
        portal: &Portal<Self::Statement>,
    ) -> PgWireResult<DescribePortalResponse>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        let statement = Statement::parse(&portal.statement.statement)?;

        Ok(DescribePortalResponse::new(
            statement.columns(&portal.result_column_format),
        ))
    }
}

/// One client connection's handlers.
struct Handlers {
    cluster: Arc<Cluster>,
}

impl PgWireServerHandlers for Handlers {
    fn extended_query_handler(&self) -> Arc<impl ExtendedQueryHandler> {
        self.cluster.clone()
    }

    fn startup_handler(&self) -> Arc<impl StartupHandler> {
        let mut scram = ScramAuth::new(self.cluster.clone());
        scram.set_iterations(SCRAM_ITERATIONS);
        let mut parameters = DefaultServerParameterProvider::default();
        parameters.server_version = SERVER_VERSION.to_owned();

        Arc::new(SASLAuthStartupHandler::new(Arc::new(parameters)).with_scram(scram))
    }

    fn error_handler(&self) -> Arc<impl ErrorHandler> {
        Arc::new(PostgresWording)
    }
}

/// Rewords pgwire's errors as PostgreSQL words them.
struct PostgresWording;

impl ErrorHandler for PostgresWording {
    fn on_error<C>(&self, _client: &C, error: &mut PgWireError)
    where
        C: ClientInfo,
    {
        if let PgWireError::InvalidPassword(user) = error {
            *error = self::error(
                "FATAL",
                "28P01",
                format!("password authentication failed for user \"{user}\""),
            );
        }
    }
}

fn error(severity: &str, code: &str, message: String) -> PgWireError {
    PgWireError::UserError(Box::new(ErrorInfo::new(
        severity.to_owned(),
        code.to_owned(),
        message,
    )))
}

/// PV never sends anything else.
fn unsupported(sql: &str) -> PgWireError {
    error(
        "ERROR",
        "0A000",
        format!("the pv-fake postgres persona doesn't run {sql:?}"),
    )
}
