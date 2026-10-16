use std::fmt::Display;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::time::Duration;

use async_io::Async;
use camino::Utf8Path;
use data_encoding::HEXLOWER;
use futures_lite::io::{AsyncReadExt, AsyncWriteExt};
use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{DNSClass, Name, RData, RecordType};
use hickory_proto::serialize::binary::BinEncodable;
use state::fs;
use zbus::Connection;
use zbus::zvariant::OwnedObjectPath;

use crate::PlatformError;
use crate::systemd::{native_proxy, within};

const STUB: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 53), 53));
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) fn check_stub() -> Result<(), PlatformError> {
    async_io::block_on(within(Duration::from_secs(10), async {
        let connection = Connection::system().await.map_err(resolved_error)?;
        let manager = native_proxy(
            &connection,
            "/org/freedesktop/systemd1"
                .try_into()
                .map_err(resolved_error)?,
            "org.freedesktop.systemd1.Manager",
        )
        .await
        .map_err(resolved_error)?;
        // Observe an already loaded service without loading or activating resolved.
        let path: OwnedObjectPath = manager
            .call("GetUnit", &("systemd-resolved.service",))
            .await
            .map_err(resolved_error)?;
        let unit = native_proxy(&connection, path, "org.freedesktop.systemd1.Unit")
            .await
            .map_err(resolved_error)?;
        let active: String = unit
            .get_property("ActiveState")
            .await
            .map_err(resolved_error)?;
        if active != "active" {
            return Err(resolved_error(format!("service is {active}")));
        }
        probe("localhost").await
    }))
    .ok_or_else(|| resolved_error("inspection timed out after 10 seconds"))?
}

async fn probe(suffix: &str) -> Result<(), PlatformError> {
    let content = fs::read_to_string(Utf8Path::new("/etc/resolv.conf")).map_err(resolved_error)?;
    let tcp = stub_transport(&content)?;
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(resolved_error)?;
    let mut request = Message::new(
        u16::from_be_bytes([nonce[0], nonce[1]]),
        MessageType::Query,
        OpCode::Query,
    );
    request.metadata.recursion_desired = true;
    request.add_query(Query::query(
        Name::from_ascii(format!("pv-{}.{suffix}.", HEXLOWER.encode(&nonce)))
            .map_err(resolved_error)?,
        RecordType::A,
    ));
    let bytes = request.to_bytes().map_err(resolved_error)?;
    let response = exchange(STUB, tcp, &bytes).await.map_err(resolved_error)?;
    validate_response(&request, &response)
}

fn stub_transport(content: &str) -> Result<bool, PlatformError> {
    let mut nameservers = Vec::new();
    let mut tcp = false;
    // Match glibc's column-zero directives and space/tab token boundaries.
    for line in content.split('\n') {
        if let Some((directive, value)) = line.split_once([' ', '\t']) {
            let mut words = value.split([' ', '\t']).filter(|word| !word.is_empty());
            match directive {
                "nameserver" => nameservers.push(words.next().unwrap_or_default()),
                "options" => tcp |= words.any(|word| word.starts_with("use-vc")),
                _ => {}
            }
        }
    }
    if nameservers != ["127.0.0.53"] {
        return Err(PlatformError::SystemIntegration(format!(
            "/etc/resolv.conf must list only nameserver 127.0.0.53, found {nameservers:?}; link it to /run/systemd/resolve/stub-resolv.conf before running `pv setup`"
        )));
    }
    Ok(tcp)
}

async fn exchange(address: SocketAddr, tcp: bool, request: &[u8]) -> io::Result<Vec<u8>> {
    within(EXCHANGE_TIMEOUT, async {
        if tcp {
            let mut stream = Async::<TcpStream>::connect(address).await?;
            let length = u16::try_from(request.len()).map_err(io::Error::other)?;
            stream.write_all(&length.to_be_bytes()).await?;
            stream.write_all(request).await?;
            let mut length = [0; 2];
            stream.read_exact(&mut length).await?;
            let mut response = vec![0; usize::from(u16::from_be_bytes(length))];
            stream.read_exact(&mut response).await?;
            Ok(response)
        } else {
            let socket = Async::<UdpSocket>::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
            // Connecting filters replies to this peer without a separate address check.
            socket.get_ref().connect(address)?;
            socket.send(request).await?;
            let mut response = vec![0; u16::MAX as usize];
            let length = socket.recv(&mut response).await?;
            response.truncate(length);
            Ok(response)
        }
    })
    .await
    .ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "DNS exchange timed out after 2 seconds",
        )
    })?
}

fn validate_response(request: &Message, bytes: &[u8]) -> Result<(), PlatformError> {
    let response = Message::from_vec(bytes).map_err(resolved_error)?;
    if response.metadata.id == request.metadata.id
        && response.metadata.message_type == MessageType::Response
        && response.metadata.op_code == request.metadata.op_code
        && response.metadata.response_code == ResponseCode::NoError
        && !response.metadata.truncation
        && response.queries == request.queries
        && response.answers.iter().any(|answer| {
            &answer.name == request.queries[0].name()
                && answer.dns_class == DNSClass::IN
                && matches!(&answer.data, RData::A(address) if address.0 == Ipv4Addr::LOCALHOST)
        })
    {
        return Ok(());
    }
    Err(resolved_error(
        "stub did not return the expected loopback A answer",
    ))
}

fn resolved_error(error: impl Display) -> PlatformError {
    PlatformError::SystemIntegration(format!(
        "could not verify the systemd-resolved stub: {error}; install systemd-resolved (Debian: `sudo apt install systemd-resolved`), enable it and check its DNSStubListener and port 53 before running `pv setup`"
    ))
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, UdpSocket};
    use std::time::Duration;

    use async_io::{Async, Timer};
    use futures_lite::future::zip;
    use futures_lite::io::{AsyncReadExt, AsyncWriteExt};
    use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
    use hickory_proto::rr::rdata::{A, AAAA};
    use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};
    use hickory_proto::serialize::binary::BinEncodable;
    use insta::assert_snapshot;

    use super::{exchange, stub_transport, validate_response};
    use crate::systemd::within;

    #[test]
    fn resolver_configuration_requires_only_the_stub_and_selects_its_transport() {
        let rows: Vec<_> = [
            ("UDP", "nameserver 127.0.0.53\n"),
            ("TCP", "nameserver 127.0.0.53\noptions edns0 use-vc\n"),
            ("comments", "# nameserver 1.1.1.1\nnameserver 127.0.0.53 ; comment\n# options use-vc\n"),
            ("search and options", "search example.test\noptions use-vc\noptions edns0\nnameserver 127.0.0.53 # comment\n"),
            ("indented nameserver", " nameserver 127.0.0.53\n"),
            ("attached hash", "nameserver 127.0.0.53#suffix\n"),
            ("attached semicolon", "nameserver 127.0.0.53;suffix\n"),
            ("CRLF nameserver", "nameserver 127.0.0.53\r\n"),
            ("options after hash", "nameserver 127.0.0.53\noptions # use-vc\n"),
            ("indented options", "nameserver 127.0.0.53\n options use-vc\n"),
            ("option prefix", "nameserver 127.0.0.53\noptions use-vc-suffix\n"),
            ("missing", "search example.test\n"),
            ("uplink", "nameserver 1.1.1.1\n"),
            ("extra resolver", "nameserver 127.0.0.53\nnameserver 1.1.1.1\n"),
            ("duplicate stub", "nameserver 127.0.0.53\nnameserver 127.0.0.53\n"),
            ("proxy stub", "nameserver 127.0.0.54\n"),
            ("empty nameserver", "nameserver\n"),
        ].into_iter().map(|(name, content)| format!("{name}: {:?}", stub_transport(content).ok())).collect();
        assert_snapshot!(rows.join("\n"), @r#"
        UDP: Some(false)
        TCP: Some(true)
        comments: Some(false)
        search and options: Some(true)
        indented nameserver: None
        attached hash: None
        attached semicolon: None
        CRLF nameserver: None
        options after hash: Some(true)
        indented options: Some(false)
        option prefix: Some(true)
        missing: None
        uplink: None
        extra resolver: None
        duplicate stub: None
        proxy stub: None
        empty nameserver: None
        "#);
    }

    #[test]
    fn stub_response_requires_a_matching_successful_loopback_answer() -> anyhow::Result<()> {
        let name = Name::from_ascii("pv-nonce.localhost.")?;
        let mut request = Message::new(42, MessageType::Query, OpCode::Query);
        request.add_query(Query::query(name.clone(), RecordType::A));
        let mut valid = Message::response(42, OpCode::Query);
        valid.add_queries(request.queries.clone());
        valid.add_answer(Record::from_rdata(name, 0, RData::A(A::new(127, 0, 0, 1))));
        type ResponseMutation = fn(&mut Message);
        let mutations: [(&str, ResponseMutation); 16] = [
            ("valid", |_| {}),
            ("wrong ID", |response| response.metadata.id += 1),
            ("query", |response| {
                response.metadata.message_type = MessageType::Query
            }),
            ("wrong opcode", |response| {
                response.metadata.op_code = OpCode::Status
            }),
            ("SERVFAIL", |response| {
                response.metadata.response_code = ResponseCode::ServFail
            }),
            ("truncated", |response| response.metadata.truncation = true),
            ("missing question", |response| response.queries.clear()),
            ("extra question", |response| {
                response.queries.push(response.queries[0].clone())
            }),
            ("wrong question name", |response| {
                response.queries[0].set_name(Name::root());
            }),
            ("wrong question type", |response| {
                response.queries[0].set_query_type(RecordType::AAAA);
            }),
            ("wrong question class", |response| {
                response.queries[0].set_query_class(DNSClass::CH);
            }),
            ("no answer", |response| response.answers.clear()),
            ("unrelated answer", |response| {
                response.answers[0].name = Name::root();
            }),
            ("wrong address", |response| {
                response.answers[0].data = RData::A(A::new(127, 0, 0, 2));
            }),
            ("wrong answer type", |response| {
                response.answers[0].data = RData::AAAA(AAAA::new(0, 0, 0, 0, 0, 0, 0, 1));
            }),
            ("wrong answer class", |response| {
                response.answers[0].dns_class = DNSClass::CH;
            }),
        ];
        let mut rows = Vec::new();
        for (name, mutate) in mutations {
            let mut response = valid.clone();
            mutate(&mut response);
            rows.push(format!(
                "{name}: {}",
                validate_response(&request, &response.to_bytes()?).is_ok()
            ));
        }
        rows.push(format!(
            "malformed packet: {}",
            validate_response(&request, &[0, 1, 2]).is_ok()
        ));
        assert_snapshot!(rows.join("\n"), @r#"
        valid: true
        wrong ID: false
        query: false
        wrong opcode: false
        SERVFAIL: false
        truncated: false
        missing question: false
        extra question: false
        wrong question name: false
        wrong question type: false
        wrong question class: false
        no answer: false
        unrelated answer: false
        wrong address: false
        wrong answer type: false
        wrong answer class: false
        malformed packet: false
        "#);
        Ok(())
    }

    #[test]
    fn stub_exchange_uses_udp_and_tcp_framing() -> anyhow::Result<()> {
        async_io::block_on(async {
            let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, pv_fake::port()?));
            let udp = Async::<UdpSocket>::bind(address)?;
            let tcp = Async::<TcpListener>::bind(address)?;
            for use_tcp in [false, true] {
                let server = within(Duration::from_secs(3), async {
                    if use_tcp {
                        let (mut stream, _) = tcp.accept().await?;
                        let mut request = [0; 5];
                        stream.read_exact(&mut request).await?;
                        assert_eq!(&request, b"\0\x03ask");
                        stream.write_all(b"\0\x03yes").await?;
                    } else {
                        let mut request = [0; 3];
                        let (length, peer) = udp.recv_from(&mut request).await?;
                        assert_eq!(length, 3);
                        assert_eq!(&request, b"ask");
                        udp.send_to(b"yes", peer).await?;
                    }
                    Ok::<_, io::Error>(())
                });
                let (server, response) = zip(server, exchange(address, use_tcp, b"ask")).await;
                server.ok_or_else(|| anyhow::anyhow!("fixture responder timed out"))??;
                assert_eq!(response?, b"yes");
            }
            Ok(())
        })
    }

    #[test]
    fn stub_exchange_deadline_covers_udp_silence_and_partial_tcp_reads() -> anyhow::Result<()> {
        async_io::block_on(async {
            let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, pv_fake::port()?));
            let udp = Async::<UdpSocket>::bind(address)?;
            let tcp = Async::<TcpListener>::bind(address)?;
            for use_tcp in [false, true] {
                let server = within(Duration::from_secs(4), async {
                    if use_tcp {
                        let (mut stream, _) = tcp.accept().await?;
                        let mut request = [0; 5];
                        stream.read_exact(&mut request).await?;
                        assert_eq!(&request, b"\0\x03ask");
                        stream.write_all(b"\0\x04").await?;
                        for _ in 0..4 {
                            Timer::after(Duration::from_millis(750)).await;
                            if let Err(error) = stream.write_all(b"x").await {
                                assert!(matches!(
                                    error.kind(),
                                    io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
                                ));
                                break;
                            }
                        }
                    } else {
                        let mut request = [0; 3];
                        udp.recv_from(&mut request).await?;
                        Timer::after(Duration::from_secs(3)).await;
                    }
                    Ok::<_, io::Error>(())
                });
                let (server, response) = zip(server, exchange(address, use_tcp, b"ask")).await;
                server.ok_or_else(|| anyhow::anyhow!("fixture responder timed out"))??;
                let Err(error) = response else {
                    anyhow::bail!("a silent or trickling responder must time out");
                };
                assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            }
            Ok(())
        })
    }
}
