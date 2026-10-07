//! The `mysqld` persona: MySQL's data directory initialization and process behavior, following
//! recordings of MySQL 8.4.9 started by PV.
//!
//! It speaks no MySQL protocol. Its port accepts connections, which is all PV's test-only
//! `RecordingMysqlAdmin` checks. `opensrv-mysql` answered PV's `sqlx` calls, but its
//! `mysql_common` dependency switches `flate2` to the C zlib backend for every crate in a
//! workspace test build, which changes other crates' archive bytes.

use std::io::{self, Write};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use tokio::net::TcpListener;

use crate::accept;

/// The system databases `--initialize-insecure` creates; PV checks for `mysql`.
const SYSTEM_DATABASES: [&str; 3] = ["mysql", "performance_schema", "sys"];

/// What `mysqld` was asked to do.
pub(crate) enum Mysqld {
    /// `--initialize-insecure` finished with this exit code.
    Initialized(u8),
    /// Serving until SIGTERM. MySQL ignores SIGINT.
    Serving,
}

/// Handles `mysqld --no-defaults --initialize-insecure --datadir <dir> --basedir <dir>`, and
/// `mysqld --no-defaults --datadir <dir> --bind-address=<address> --port <port> --mysqlx=0
/// --socket <path> --init-file <path>`, which listens on TCP only.
pub(crate) async fn start(argv: &[String]) -> Result<Mysqld> {
    let options = Options::parse(argv)?;
    let Some(data_dir) = options.data_dir else {
        bail!("expected `mysqld --datadir <dir>`");
    };
    if options.initialize {
        return Ok(Mysqld::Initialized(initialize(&data_dir)?));
    }
    let (Some(bind_address), Some(port)) = (options.bind_address, options.port) else {
        bail!("expected `mysqld --bind-address=<address> --port <port>`");
    };
    if !state::fs::path_is_directory(&data_dir.join("mysql"))? {
        bail!("{data_dir} isn't initialized; run `mysqld --initialize-insecure` first");
    }
    let listener = TcpListener::bind((bind_address.as_str(), port))
        .await
        .with_context(|| format!("binding MySQL port {port}"))?;
    tokio::spawn(async move {
        loop {
            drop(accept(&listener).await);
        }
    });

    Ok(Mysqld::Serving)
}

#[derive(Default)]
struct Options {
    initialize: bool,
    data_dir: Option<Utf8PathBuf>,
    bind_address: Option<String>,
    port: Option<u16>,
}

impl Options {
    fn parse(argv: &[String]) -> Result<Self> {
        // mysqld only reads --no-defaults as its first argument and refuses it anywhere else.
        if argv
            .iter()
            .skip(2)
            .any(|argument| argument == "--no-defaults")
        {
            bail!("--no-defaults must be mysqld's first argument");
        }
        let mut options = Self::default();
        let mut arguments = argv.iter().skip(1);
        while let Some(argument) = arguments.next() {
            let (flag, inline_value) = match argument.split_once('=') {
                Some((flag, value)) => (flag, Some(value.to_owned())),
                None => (argument.as_str(), None),
            };
            match flag {
                "--no-defaults" => continue,
                "--initialize-insecure" => {
                    options.initialize = true;
                    continue;
                }
                _ => {}
            }
            let Some(value) = inline_value.or_else(|| arguments.next().cloned()) else {
                bail!("mysqld {flag} needs a value");
            };
            match flag {
                "--datadir" => options.data_dir = Some(value.into()),
                "--bind-address" => options.bind_address = Some(value),
                "--port" => {
                    options.port = Some(value.parse().with_context(|| format!("port {value}"))?);
                }
                // The persona signs no one in and serves TCP only: no accounts, Unix socket or
                // X Protocol.
                "--basedir" | "--init-file" | "--socket" | "--mysqlx" => {}
                _ => bail!("unexpected mysqld argument {argument}"),
            }
        }

        Ok(options)
    }
}

fn initialize(data_dir: &Utf8Path) -> Result<u8> {
    if state::fs::path_is_directory(data_dir)? && !state::fs::read_dir_paths(data_dir)?.is_empty() {
        // mysqld's errors for a data directory that isn't empty, without the timestamp and thread.
        let _write_result = write!(
            io::stderr(),
            "[ERROR] [MY-010457] [Server] --initialize specified but the data directory has files \
             in it. Aborting.\n[ERROR] [MY-013236] [Server] The designated data directory \
             {data_dir}/ is unusable. You can remove all files that the server added to \
             it.\n[ERROR] [MY-010119] [Server] Aborting\n"
        );
        return Ok(1);
    }
    for database in SYSTEM_DATABASES {
        state::fs::ensure_user_dir(&data_dir.join(database))?;
    }

    Ok(0)
}
