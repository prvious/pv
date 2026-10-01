//! The `redis_server` persona: the parts of Redis that PV's readiness check uses. Replies follow
//! recordings of Redis 8.8.0 started with PV's rendered config.

use std::io::{self, Write};

use anyhow::{Context, Result, bail};
use camino::Utf8PathBuf;
use redis_protocol::resp2::decode::decode;
use redis_protocol::resp2::encode::encode;
use redis_protocol::resp2::types::{OwnedFrame, Resp2Frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::accept;

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
    tokio::spawn(async move {
        loop {
            tokio::spawn(serve_client(accept(&listener).await));
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

async fn serve_client(mut stream: TcpStream) {
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
            let Some((reply, close)) = reply(&frame) else {
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

/// Redis's reply to one command, and whether to close the connection after it.
fn reply(frame: &OwnedFrame) -> Option<(OwnedFrame, bool)> {
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

fn error(message: &str) -> OwnedFrame {
    OwnedFrame::Error(message.to_owned())
}
