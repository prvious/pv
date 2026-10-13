//! The `redis_server` persona: the parts of Redis that PV's readiness check and allocation cleanup
//! use. Replies follow recordings of Redis 8.8.0 started with PV's rendered config. Keys live in
//! memory for as long as the persona runs, in Redis's default 16 logical databases.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::{Context, Result, bail};
use camino::Utf8PathBuf;
use redis_protocol::resp2::decode::decode;
use redis_protocol::resp2::encode::encode;
use redis_protocol::resp2::types::{OwnedFrame, Resp2Frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::accept;

/// Redis's default `databases` setting, which PV's config keeps.
const DATABASES: usize = 16;

/// Each logical database's keys and values.
type Keyspace = Arc<Mutex<Vec<BTreeMap<String, String>>>>;

/// Handles `redis-server <config>`. Returns an exit code to exit with now, or `None` once serving.
pub(crate) async fn start(argv: &[String]) -> Result<Option<u8>> {
    let Some(config_path) = argv.get(1) else {
        bail!("expected `redis-server <config>`");
    };
    let config = state::fs::read_to_string(&Utf8PathBuf::from(config_path))?;
    let (mut port, mut dir) = (None, None);
    for (line_number, line) in config.lines().enumerate() {
        match line.trim().split_once(' ') {
            Some(("port", value)) => {
                port = Some(
                    value
                        .parse::<u16>()
                        .with_context(|| format!("port {value}"))?,
                );
            }
            Some(("dir", value)) => dir = Some((line_number + 1, line.trim(), unquote(value)?)),
            _ => {}
        }
    }
    if let Some((line_number, line, dir)) = dir
        && !state::fs::path_is_directory(&dir)?
    {
        // Redis's message for a `dir` that doesn't exist.
        let _write_result = write!(
            io::stderr(),
            "\n*** FATAL CONFIG FILE ERROR (Redis 8.8.0) ***\nReading the configuration file, at \
             line {line_number}\n>>> '{line}'\nNo such file or directory\n"
        );
        return Ok(Some(1));
    }
    let Some(port) = port else {
        bail!("{config_path} has no `port`");
    };
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("binding Redis port {port}"))?;
    let keyspace = Keyspace::new(Mutex::new(vec![BTreeMap::new(); DATABASES]));
    tokio::spawn(async move {
        loop {
            tokio::spawn(serve_client(accept(&listener).await, keyspace.clone()));
        }
    });

    Ok(None)
}

/// PV renders `dir` as a JSON string, which Redis's double-quoted values accept.
fn unquote(value: &str) -> Result<Utf8PathBuf> {
    let value = value.trim();
    if value.starts_with('"') {
        return Ok(serde_json::from_str::<String>(value)
            .with_context(|| format!("dir {value}"))?
            .into());
    }

    Ok(value.into())
}

async fn serve_client(mut stream: TcpStream, keyspace: Keyspace) {
    let mut database = 0;
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let Ok(read) = stream.read(&mut chunk).await else {
            return;
        };
        if read == 0 {
            return;
        }
        buffer.extend_from_slice(&chunk[..read]);
        // Pipelined commands arrive together; answer each complete one in order.
        loop {
            let (frame, used) = match decode(&buffer) {
                Ok(Some(decoded)) => decoded,
                Ok(None) => break,
                // ponytail: PV only sends RESP arrays, so Redis's inline commands aren't
                // emulated; anything else closes the connection.
                Err(_error) => return,
            };
            buffer.drain(..used);
            let Some((reply, close)) = reply(&frame, &keyspace, &mut database) else {
                return;
            };
            let mut encoded = vec![0; reply.encode_len(false)];
            if encode(&mut encoded, &reply, false).is_err()
                || stream.write_all(&encoded).await.is_err()
                || close
            {
                return;
            }
        }
    }
}

/// Redis's reply to one command, and whether to close the connection after it. `database` is the
/// connection's selected logical database.
fn reply(
    frame: &OwnedFrame,
    keyspace: &Mutex<Vec<BTreeMap<String, String>>>,
    database: &mut usize,
) -> Option<(OwnedFrame, bool)> {
    let OwnedFrame::Array(parts) = frame else {
        return None;
    };
    let arguments = parts
        .iter()
        .filter_map(|part| part.as_bytes())
        .map(|argument| String::from_utf8_lossy(argument).into_owned())
        .collect::<Vec<_>>();
    let (command, rest) = arguments.split_first()?;

    let reply = if command.eq_ignore_ascii_case("PING") {
        match rest {
            [] => OwnedFrame::SimpleString(b"PONG".to_vec()),
            [message] => OwnedFrame::BulkString(message.clone().into_bytes()),
            _ => error("ERR wrong number of arguments for 'ping' command"),
        }
    } else if command.eq_ignore_ascii_case("CLIENT") {
        match rest.first() {
            Some(subcommand)
                if subcommand.eq_ignore_ascii_case("SETINFO")
                    || subcommand.eq_ignore_ascii_case("SETNAME") =>
            {
                OwnedFrame::SimpleString(b"OK".to_vec())
            }
            Some(subcommand) => error(&format!(
                "ERR unknown subcommand '{subcommand}'. Try CLIENT HELP."
            )),
            None => error("ERR wrong number of arguments for 'client' command"),
        }
    } else if command.eq_ignore_ascii_case("QUIT") {
        return Some((OwnedFrame::SimpleString(b"OK".to_vec()), true));
    } else if command.eq_ignore_ascii_case("SELECT") {
        match rest {
            [index] => match index.parse::<usize>() {
                Ok(index) if index < DATABASES => {
                    *database = index;
                    OwnedFrame::SimpleString(b"OK".to_vec())
                }
                Ok(_index) => error("ERR DB index is out of range"),
                Err(_error) => error("ERR value is not an integer or out of range"),
            },
            _ => error("ERR wrong number of arguments for 'select' command"),
        }
    } else if let Some(reply) = keyspace_reply(command, rest, keyspace, *database) {
        reply
    } else {
        let quoted = rest
            .iter()
            .map(|argument| format!("'{argument}' "))
            .collect::<String>();
        error(&format!(
            "ERR unknown command '{command}', with args beginning with: {quoted}"
        ))
    };

    Some((reply, false))
}

/// Replies to the key commands, or `None` for any other command.
fn keyspace_reply(
    command: &str,
    rest: &[String],
    keyspace: &Mutex<Vec<BTreeMap<String, String>>>,
    database: usize,
) -> Option<OwnedFrame> {
    let command = command.to_ascii_uppercase();
    if !matches!(command.as_str(), "SET" | "EXISTS" | "UNLINK" | "SCAN") {
        return None;
    }
    let mut keyspace = keyspace.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(keys) = keyspace.get_mut(database) else {
        return Some(error("ERR DB index is out of range"));
    };

    let reply = match (command.as_str(), rest) {
        ("SET", [key, value]) => {
            keys.insert(key.clone(), value.clone());
            OwnedFrame::SimpleString(b"OK".to_vec())
        }
        ("EXISTS", [_, ..]) => count(rest.iter().filter(|key| keys.contains_key(*key)).count()),
        ("UNLINK", [_, ..]) => count(
            rest.iter()
                .filter(|key| keys.remove(key.as_str()).is_some())
                .count(),
        ),
        ("SCAN", [cursor, options @ ..]) => scan(keys, cursor, options),
        _ => error(&format!(
            "ERR wrong number of arguments for '{}' command",
            command.to_ascii_lowercase()
        )),
    };

    Some(reply)
}

/// `SCAN <cursor> MATCH <prefix>* COUNT <count>`: the cursor is an offset into the sorted keys,
/// and each call looks at `count` keys, so a caller has to follow the cursor to the end as with
/// real Redis. Only a prefix pattern is supported, which is all PV sends.
fn scan(keys: &BTreeMap<String, String>, cursor: &str, options: &[String]) -> OwnedFrame {
    let [match_keyword, pattern, count_keyword, count] = options else {
        return error("ERR syntax error");
    };
    let (Ok(cursor), Ok(count), Some(prefix)) = (
        cursor.parse::<usize>(),
        count.parse::<usize>(),
        pattern.strip_suffix('*'),
    ) else {
        return error("ERR syntax error");
    };
    if !match_keyword.eq_ignore_ascii_case("MATCH")
        || !count_keyword.eq_ignore_ascii_case("COUNT")
        || count == 0
        || prefix.contains(['*', '?', '[', '\\'])
    {
        return error("ERR syntax error");
    }

    let scanned = keys.keys().skip(cursor).take(count).collect::<Vec<_>>();
    let next = if cursor + scanned.len() >= keys.len() {
        0
    } else {
        cursor + scanned.len()
    };
    let matches = scanned
        .into_iter()
        .filter(|key| key.starts_with(prefix))
        .map(|key| OwnedFrame::BulkString(key.clone().into_bytes()))
        .collect();

    OwnedFrame::Array(vec![
        OwnedFrame::BulkString(next.to_string().into_bytes()),
        OwnedFrame::Array(matches),
    ])
}

fn count(count: usize) -> OwnedFrame {
    OwnedFrame::Integer(i64::try_from(count).unwrap_or(i64::MAX))
}

fn error(message: &str) -> OwnedFrame {
    OwnedFrame::Error(message.to_owned())
}
