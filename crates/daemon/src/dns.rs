use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, UdpSocket};
use std::time::Duration;

use hickory_proto::op::{Message, ResponseCode};
use hickory_proto::rr::rdata::{A, AAAA};
use hickory_proto::rr::{Name, RData, Record, RecordType};
use hickory_proto::serialize::binary::BinEncodable;
use state::{Database, PortOwner, PortRequest, PvPaths};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener as TokioTcpListener, TcpStream, UdpSocket as TokioUdpSocket};
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::sleep;

use crate::DaemonError;

pub const DNS_TTL_SECONDS: u32 = 5;
const DNS_TCP_ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(50);
const DNS_BIND_ATTEMPTS: usize = 10;
const UDP_PACKET_BUFFER_BYTES: usize = 512;

#[derive(Debug)]
pub struct RunningDnsResolver {
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<(), DaemonError>>,
}

impl RunningDnsResolver {
    pub async fn start(paths: PvPaths) -> Result<Self, DaemonError> {
        let (udp_socket, tcp_listener) = bind_assigned_dns_sockets(&paths).await?;
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(run_dns_resolver(
            udp_socket,
            tcp_listener,
            shutdown_receiver,
        ));

        Ok(Self {
            shutdown: Some(shutdown),
            task,
        })
    }

    pub async fn shutdown(mut self) -> Result<(), DaemonError> {
        self.signal_shutdown();
        self.wait_for_completion().await
    }

    pub async fn wait_for_completion(&mut self) -> Result<(), DaemonError> {
        (&mut self.task).await?
    }

    fn signal_shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }

    #[cfg(test)]
    pub(crate) fn pending_for_test() -> Self {
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(async {
            let _ = shutdown_receiver.await;
            Ok(())
        });

        Self {
            shutdown: Some(shutdown),
            task,
        }
    }

    #[cfg(test)]
    pub(crate) fn aborted_for_test() -> Self {
        let resolver = Self::pending_for_test();
        resolver.task.abort();

        resolver
    }

    #[cfg(test)]
    pub(crate) fn failed_for_test(error: io::Error) -> Self {
        let (shutdown, _shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(async { Err(DaemonError::Io(error)) });

        Self {
            shutdown: Some(shutdown),
            task,
        }
    }
}

async fn bind_assigned_dns_sockets(
    paths: &PvPaths,
) -> Result<(TokioUdpSocket, TokioTcpListener), DaemonError> {
    let mut last_bind_error = None;

    for _attempt in 0..DNS_BIND_ATTEMPTS {
        let assignment = {
            let mut database = Database::open(paths)?;
            database.assign_port(PortRequest::pv_dns(), dns_port_available)?
        };

        match bind_dns_sockets(assignment.port).await {
            Ok(sockets) => return Ok(sockets),
            Err(error) if dns_bind_error_is_addr_in_use(&error) => {
                let mut database = Database::open(paths)?;
                database.release_port(PortOwner::Dns)?;
                last_bind_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }

    if let Some(error) = last_bind_error {
        return Err(error);
    }

    let assignment = {
        let mut database = Database::open(paths)?;
        database.assign_port(PortRequest::pv_dns(), dns_port_available)?
    };
    bind_dns_sockets(assignment.port).await
}

async fn bind_dns_sockets(port: u16) -> Result<(TokioUdpSocket, TokioTcpListener), DaemonError> {
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let udp_socket =
        TokioUdpSocket::bind(address)
            .await
            .map_err(|source| DaemonError::DnsBind {
                protocol: "UDP",
                port,
                source,
            })?;
    let tcp_listener =
        TokioTcpListener::bind(address)
            .await
            .map_err(|source| DaemonError::DnsBind {
                protocol: "TCP",
                port,
                source,
            })?;

    Ok((udp_socket, tcp_listener))
}

fn dns_bind_error_is_addr_in_use(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::DnsBind { source, .. } if source.kind() == io::ErrorKind::AddrInUse
    )
}

pub fn response_bytes(request: &[u8]) -> Result<Vec<u8>, DaemonError> {
    let request = Message::from_vec(request)?;
    let mut response = Message::response(request.metadata.id, request.metadata.op_code);
    response.metadata.recursion_desired = request.metadata.recursion_desired;
    response.metadata.authoritative = true;
    response.metadata.recursion_available = false;
    response.metadata.response_code = ResponseCode::NoError;
    response.add_queries(request.queries.iter().cloned());

    for query in &request.queries {
        if !is_test_name(query.name()) {
            continue;
        }

        match query.query_type() {
            RecordType::A => {
                response.add_answer(Record::from_rdata(
                    query.name().clone(),
                    DNS_TTL_SECONDS,
                    RData::A(A::new(127, 0, 0, 1)),
                ));
            }
            RecordType::AAAA => {
                response.add_answer(Record::from_rdata(
                    query.name().clone(),
                    DNS_TTL_SECONDS,
                    RData::AAAA(AAAA::new(0, 0, 0, 0, 0, 0, 0, 1)),
                ));
            }
            _ => {}
        }
    }

    Ok(response.to_bytes()?)
}

async fn run_dns_resolver(
    udp_socket: TokioUdpSocket,
    tcp_listener: TokioTcpListener,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), DaemonError> {
    let mut udp_buffer = vec![0; UDP_PACKET_BUFFER_BYTES];
    let mut tcp_connections = JoinSet::new();

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tcp_connections.abort_all();
                while tcp_connections.join_next().await.is_some() {}

                return Ok(());
            }
            received = udp_socket.recv_from(&mut udp_buffer) => {
                let (length, address) = received?;
                handle_udp_datagram(&udp_socket, &udp_buffer[..length], address).await;
            }
            accepted = tcp_listener.accept() => {
                match accepted {
                    Ok((stream, _address)) => {
                        tcp_connections.spawn(handle_tcp_connection(stream));
                    }
                    Err(_error) => {
                        sleep(DNS_TCP_ACCEPT_ERROR_BACKOFF).await;
                    }
                }
            }
            joined = tcp_connections.join_next(), if !tcp_connections.is_empty() => {
                match joined {
                    Some(Ok(Ok(()))) | None => {}
                    Some(Ok(Err(_error))) => {}
                    Some(Err(error)) if error.is_panic() => return Err(error.into()),
                    Some(Err(_error)) => {}
                }
            }
        }
    }
}

async fn handle_udp_datagram(socket: &TokioUdpSocket, request: &[u8], address: SocketAddr) {
    let Ok(response) = response_bytes(request) else {
        return;
    };

    let _send_result = socket.send_to(&response, address).await;
}

async fn handle_tcp_connection(mut stream: TcpStream) -> Result<(), DaemonError> {
    let mut length_prefix = [0; 2];
    stream.read_exact(&mut length_prefix).await?;
    let request_length = usize::from(u16::from_be_bytes(length_prefix));
    let mut request = vec![0; request_length];
    stream.read_exact(&mut request).await?;

    let response = response_bytes(&request)?;
    let response_length = u16::try_from(response.len()).map_err(|_error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "DNS TCP response exceeded 65535 bytes",
        )
    })?;
    stream.write_all(&response_length.to_be_bytes()).await?;
    stream.write_all(&response).await?;
    stream.shutdown().await?;

    Ok(())
}

pub fn dns_port_available(port: u16) -> bool {
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let Ok(_udp_socket) = UdpSocket::bind(address) else {
        return false;
    };

    TcpListener::bind(address).is_ok()
}

fn is_test_name(name: &Name) -> bool {
    let ascii_name = name.to_ascii();
    let normalized_name = ascii_name.trim_end_matches('.').to_ascii_lowercase();

    normalized_name == "test" || normalized_name.ends_with(".test")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use anyhow::{Result, anyhow};
    use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
    use hickory_proto::rr::rdata::{A, AAAA};
    use hickory_proto::rr::{DNSClass, Name, RData, RecordType};
    use hickory_proto::serialize::binary::BinEncodable;

    const REQUEST_ID: u16 = 42;
    const EXPECTED_DNS_TTL_SECONDS: u32 = 5;

    #[test]
    fn builds_a_and_aaaa_loopback_answers_for_test_names() -> Result<()> {
        let a_response = response_for("acme.test.", RecordType::A)?;
        assert_common_response_fields(&a_response, "acme.test.", RecordType::A)?;
        assert_eq!(a_response.answers.len(), 1);
        let a_answer = &a_response.answers[0];
        assert_eq!(&a_answer.name, &Name::from_str("acme.test.")?);
        assert_eq!(a_answer.record_type(), RecordType::A);
        assert_eq!(a_answer.dns_class, DNSClass::IN);
        assert_eq!(a_answer.ttl, EXPECTED_DNS_TTL_SECONDS);
        assert_eq!(&a_answer.data, &RData::A(A::new(127, 0, 0, 1)));

        let aaaa_response = response_for("acme.test.", RecordType::AAAA)?;
        assert_common_response_fields(&aaaa_response, "acme.test.", RecordType::AAAA)?;
        assert_eq!(aaaa_response.answers.len(), 1);
        let aaaa_answer = &aaaa_response.answers[0];
        assert_eq!(&aaaa_answer.name, &Name::from_str("acme.test.")?);
        assert_eq!(aaaa_answer.record_type(), RecordType::AAAA);
        assert_eq!(aaaa_answer.dns_class, DNSClass::IN);
        assert_eq!(aaaa_answer.ttl, EXPECTED_DNS_TTL_SECONDS);
        assert_eq!(
            &aaaa_answer.data,
            &RData::AAAA(AAAA::new(0, 0, 0, 0, 0, 0, 0, 1))
        );

        Ok(())
    }

    #[test]
    fn returns_nodata_for_unsupported_or_non_test_queries() -> Result<()> {
        let mx_response = response_for("acme.test.", RecordType::MX)?;
        assert_common_response_fields(&mx_response, "acme.test.", RecordType::MX)?;
        assert!(mx_response.answers.is_empty());

        let non_test_response = response_for("example.com.", RecordType::A)?;
        assert_common_response_fields(&non_test_response, "example.com.", RecordType::A)?;
        assert!(non_test_response.answers.is_empty());

        Ok(())
    }

    fn response_for(name: &str, record_type: RecordType) -> Result<Message> {
        let request = request_bytes(name, record_type)?;
        let response = super::response_bytes(&request)?;

        Ok(Message::from_vec(&response)?)
    }

    fn request_bytes(name: &str, record_type: RecordType) -> Result<Vec<u8>> {
        let query = Query::query(Name::from_str(name)?, record_type);
        let mut message = Message::new(REQUEST_ID, MessageType::Query, OpCode::Query);
        message.metadata.recursion_desired = true;
        message.add_query(query);

        Ok(message.to_bytes()?)
    }

    fn assert_common_response_fields(
        response: &Message,
        name: &str,
        record_type: RecordType,
    ) -> Result<()> {
        assert_eq!(response.metadata.id, REQUEST_ID);
        assert_eq!(response.metadata.message_type, MessageType::Response);
        assert_eq!(response.metadata.op_code, OpCode::Query);
        assert!(response.metadata.recursion_desired);
        assert!(response.metadata.authoritative);
        assert!(!response.metadata.recursion_available);
        assert_eq!(response.metadata.response_code, ResponseCode::NoError);
        assert_eq!(response.queries.len(), 1);

        let Some(query) = response.queries.first() else {
            return Err(anyhow!("response did not preserve the query section"));
        };
        assert_eq!(query.name(), &Name::from_str(name)?);
        assert_eq!(query.query_type(), record_type);
        assert_eq!(query.query_class(), DNSClass::IN);

        Ok(())
    }
}
