use futures_util::SinkExt;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LinesCodec};

use crate::error::ProtocolError;

const MAX_PROTOCOL_LINE_BYTES: usize = 64 * 1024;

pub type DaemonTransport<Stream> = Framed<Stream, LinesCodec>;

pub fn transport<Stream>(stream: Stream) -> DaemonTransport<Stream>
where
    Stream: AsyncRead + AsyncWrite,
{
    Framed::new(
        stream,
        LinesCodec::new_with_max_length(MAX_PROTOCOL_LINE_BYTES),
    )
}

pub async fn write_line<Stream>(
    transport: &mut DaemonTransport<Stream>,
    line: &impl Serialize,
) -> Result<(), ProtocolError>
where
    Stream: AsyncWrite + Unpin,
{
    let encoded = serde_json::to_string(line)?;

    transport.send(encoded).await?;

    Ok(())
}
