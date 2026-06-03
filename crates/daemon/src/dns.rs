use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};

use hickory_proto::op::{Message, ResponseCode};
use hickory_proto::rr::rdata::{A, AAAA};
use hickory_proto::rr::{Name, RData, Record, RecordType};
use hickory_proto::serialize::binary::BinEncodable;

use crate::DaemonError;

pub const DNS_TTL_SECONDS: u32 = 5;

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
