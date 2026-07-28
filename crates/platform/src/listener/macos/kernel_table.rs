use std::collections::BTreeSet;
use std::ffi::CStr;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::ptr;

use thiserror::Error;

const TCP_PCBLIST_NAME: &CStr = c"net.inet.tcp.pcblist_n";
const MAX_ATTEMPTS: usize = 3;

// These offsets follow Apple's private `xinpcb_n`, `xtcpcb_n`, and `xinpgen`
// layouts. They are unchanged in the published XNU sources for macOS 13, 14,
// 15, and 26; parsing remains bounds-checked because the ABI is not public.
const XINPGEN_LENGTH: usize = 24;
const XGEN_HEADER_LENGTH: usize = 8;
const XINPCB_MINIMUM_LENGTH: usize = 104;
const XTCPCB_MINIMUM_LENGTH: usize = 40;

const XSO_INPCB: u32 = 0x010;
const XSO_TCPCB: u32 = 0x020;
const INP_IPV4: u8 = 0x1;
const INP_IPV6: u8 = 0x2;
const TCPS_LISTEN: u32 = 1;

const INPCB_LOCAL_PORT_OFFSET: usize = 18;
const INPCB_GENERATION_OFFSET: usize = 28;
const INPCB_VERSION_FLAGS_OFFSET: usize = 44;
const INPCB_LOCAL_ADDRESS_OFFSET: usize = 64;
const INPCB_LOCAL_ADDRESS_LENGTH: usize = 16;
const INPCB_IPV4_ADDRESS_OFFSET: usize = INPCB_LOCAL_ADDRESS_OFFSET + 12;
const TCPCB_STATE_OFFSET: usize = 36;

#[derive(Debug, Error)]
pub(super) enum KernelTableError {
    #[error(transparent)]
    Fetch(#[from] FetchError),

    #[error(transparent)]
    Parse(#[from] ParseError),

    #[error("TCP listener snapshot changed during {attempts} consecutive inspections")]
    SnapshotUnstable { attempts: usize },
}

#[derive(Debug, Error)]
pub(super) enum FetchError {
    #[error("could not query the macOS TCP PCB table size: {0}")]
    Size(#[source] io::Error),

    #[error("could not read the macOS TCP PCB table: {0}")]
    Read(#[source] io::Error),

    #[error("the macOS TCP PCB table grew during {attempts} consecutive reads")]
    TableGrowth { attempts: usize },

    #[error(
        "the macOS TCP PCB table reported {actual} bytes after receiving a {capacity}-byte buffer"
    )]
    InvalidReturnedLength { capacity: usize, actual: usize },
}

#[derive(Debug, Error, Eq, PartialEq)]
pub(super) enum ParseError {
    #[error("TCP PCB table is too short: expected at least {minimum} bytes, received {actual}")]
    TableTooShort { minimum: usize, actual: usize },

    #[error("TCP PCB table {position} envelope has length {actual}; expected {expected}")]
    InvalidEnvelopeLength {
        position: &'static str,
        expected: usize,
        actual: usize,
    },

    #[error("TCP PCB record header is truncated at byte {offset}")]
    TruncatedRecordHeader { offset: usize },

    #[error("TCP PCB record at byte {offset} has invalid length {length}")]
    InvalidRecordLength { offset: usize, length: usize },

    #[error(
        "TCP PCB record at byte {offset} extends {padded_length} bytes beyond the table boundary"
    )]
    TruncatedRecord { offset: usize, padded_length: usize },

    #[error(
        "TCP PCB record kind {kind:#x} at byte {offset} is too short: expected at least {minimum} bytes, received {actual}"
    )]
    KnownRecordTooShort {
        offset: usize,
        kind: u32,
        minimum: usize,
        actual: usize,
    },

    #[error("TCP state record at byte {offset} has no preceding internet PCB record")]
    MissingInternetPcb { offset: usize },

    #[error("TCP PCB table ended with an incomplete listener record")]
    IncompleteRecord,

    #[error("TCP PCB table has no trailing snapshot envelope")]
    MissingTrailer,

    #[error(
        "TCP PCB snapshot changed from count/gen/socket-gen {header_count}/{header_generation}/{header_socket_generation} to {trailer_count}/{trailer_generation}/{trailer_socket_generation}"
    )]
    SnapshotChanged {
        header_count: u32,
        header_generation: u64,
        header_socket_generation: u64,
        trailer_count: u32,
        trailer_generation: u64,
        trailer_socket_generation: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SnapshotEnvelope {
    count: u32,
    generation: u64,
    socket_generation: u64,
}

#[derive(Clone, Copy, Debug)]
struct InternetPcb {
    generation: u64,
    version_flags: u8,
    local_port: u16,
    local_address: [u8; INPCB_LOCAL_ADDRESS_LENGTH],
}

pub(super) fn loopback_tcp_listener_ports() -> Result<BTreeSet<u16>, KernelTableError> {
    for _attempt in 1..=MAX_ATTEMPTS {
        let table = fetch_tcp_table()?;

        match parse_tcp_table(&table) {
            Ok(ports) => return Ok(ports),
            Err(ParseError::SnapshotChanged { .. }) => {}
            Err(error) => return Err(error.into()),
        }
    }

    Err(KernelTableError::SnapshotUnstable {
        attempts: MAX_ATTEMPTS,
    })
}

fn fetch_tcp_table() -> Result<Vec<u8>, FetchError> {
    fetch_tcp_table_with(query_tcp_table)
}

fn fetch_tcp_table_with(
    mut query: impl FnMut(Option<&mut [u8]>) -> io::Result<usize>,
) -> Result<Vec<u8>, FetchError> {
    for _attempt in 1..=MAX_ATTEMPTS {
        let capacity = query(None).map_err(FetchError::Size)?;
        let mut table = vec![0; capacity];

        match query(Some(&mut table)) {
            Ok(actual) if actual <= capacity => {
                table.truncate(actual);
                return Ok(table);
            }
            Ok(actual) => {
                return Err(FetchError::InvalidReturnedLength { capacity, actual });
            }
            Err(error) if error.raw_os_error() == Some(libc::ENOMEM) => {}
            Err(error) => return Err(FetchError::Read(error)),
        }
    }

    Err(FetchError::TableGrowth {
        attempts: MAX_ATTEMPTS,
    })
}

fn query_tcp_table(buffer: Option<&mut [u8]>) -> io::Result<usize> {
    let mut length = buffer.as_ref().map_or(0, |bytes| bytes.len());
    let pointer = buffer.map_or(ptr::null_mut(), |bytes| bytes.as_mut_ptr().cast());

    // SAFETY: TCP_PCBLIST_NAME is NUL-terminated, `length` points to valid writable
    // storage, and `pointer` is either null for the size query or covers `length`
    // writable bytes for the data query. Both new-value arguments are null/zero
    // because this is a read-only sysctl request.
    let status = unsafe {
        libc::sysctlbyname(
            TCP_PCBLIST_NAME.as_ptr(),
            pointer,
            &mut length,
            ptr::null_mut(),
            0,
        )
    };

    if status == 0 {
        Ok(length)
    } else {
        Err(io::Error::last_os_error())
    }
}

fn parse_tcp_table(table: &[u8]) -> Result<BTreeSet<u16>, ParseError> {
    if table.len() < XINPGEN_LENGTH {
        return Err(ParseError::TableTooShort {
            minimum: XINPGEN_LENGTH,
            actual: table.len(),
        });
    }

    let header = parse_envelope(table, "leading")?;
    if table.len() == XINPGEN_LENGTH && header.count == 0 {
        return Ok(BTreeSet::new());
    }

    let mut offset = XINPGEN_LENGTH;
    let mut pending_internet_pcb = None;
    let mut ports = BTreeSet::new();

    while offset < table.len() {
        let remaining = table.len() - offset;
        if remaining == XINPGEN_LENGTH {
            let trailer = parse_envelope(&table[offset..], "trailing")?;

            if pending_internet_pcb.is_some() {
                return Err(ParseError::IncompleteRecord);
            }
            // `xig_sogen` is global across all socket families, so unrelated
            // socket allocation or release can change it while this TCP PCB snapshot
            // remains coherent. Apple netstat likewise checks the PCB-specific generation.
            if header.generation != trailer.generation || header.count != trailer.count {
                return Err(ParseError::SnapshotChanged {
                    header_count: header.count,
                    header_generation: header.generation,
                    header_socket_generation: header.socket_generation,
                    trailer_count: trailer.count,
                    trailer_generation: trailer.generation,
                    trailer_socket_generation: trailer.socket_generation,
                });
            }

            return Ok(ports);
        }
        if remaining < XGEN_HEADER_LENGTH {
            return Err(ParseError::TruncatedRecordHeader { offset });
        }

        let length = read_native_u32(table, offset) as usize;
        if length <= XINPGEN_LENGTH {
            return Err(ParseError::InvalidRecordLength { offset, length });
        }
        let padded_length =
            length
                .checked_add(7)
                .map(|value| value & !7)
                .ok_or(ParseError::TruncatedRecord {
                    offset,
                    padded_length: length,
                })?;
        if padded_length > remaining {
            return Err(ParseError::TruncatedRecord {
                offset,
                padded_length,
            });
        }

        let record = &table[offset..offset + length];
        let kind = read_native_u32(record, 4);
        match kind {
            XSO_INPCB => {
                if pending_internet_pcb.is_some() {
                    return Err(ParseError::IncompleteRecord);
                }
                pending_internet_pcb = Some(parse_internet_pcb(record, offset)?);
            }
            XSO_TCPCB => {
                let internet_pcb = pending_internet_pcb
                    .take()
                    .ok_or(ParseError::MissingInternetPcb { offset })?;
                let state = parse_tcp_state(record, offset)?;

                if internet_pcb.generation <= header.generation
                    && state == TCPS_LISTEN
                    && occupies_loopback(internet_pcb)
                {
                    ports.insert(internet_pcb.local_port);
                }
            }
            _ => {}
        }

        offset += padded_length;
    }

    Err(ParseError::MissingTrailer)
}

fn parse_envelope(bytes: &[u8], position: &'static str) -> Result<SnapshotEnvelope, ParseError> {
    let actual = read_native_u32(bytes, 0) as usize;
    if actual != XINPGEN_LENGTH {
        return Err(ParseError::InvalidEnvelopeLength {
            position,
            expected: XINPGEN_LENGTH,
            actual,
        });
    }

    Ok(SnapshotEnvelope {
        count: read_native_u32(bytes, 4),
        generation: read_native_u64(bytes, 8),
        socket_generation: read_native_u64(bytes, 16),
    })
}

fn parse_internet_pcb(record: &[u8], offset: usize) -> Result<InternetPcb, ParseError> {
    require_record_length(record, offset, XSO_INPCB, XINPCB_MINIMUM_LENGTH)?;

    let mut local_address = [0; INPCB_LOCAL_ADDRESS_LENGTH];
    local_address.copy_from_slice(
        &record
            [INPCB_LOCAL_ADDRESS_OFFSET..INPCB_LOCAL_ADDRESS_OFFSET + INPCB_LOCAL_ADDRESS_LENGTH],
    );

    Ok(InternetPcb {
        generation: read_native_u64(record, INPCB_GENERATION_OFFSET),
        version_flags: record[INPCB_VERSION_FLAGS_OFFSET],
        local_port: read_network_u16(record, INPCB_LOCAL_PORT_OFFSET),
        local_address,
    })
}

fn parse_tcp_state(record: &[u8], offset: usize) -> Result<u32, ParseError> {
    require_record_length(record, offset, XSO_TCPCB, XTCPCB_MINIMUM_LENGTH)?;
    Ok(read_native_u32(record, TCPCB_STATE_OFFSET))
}

fn require_record_length(
    record: &[u8],
    offset: usize,
    kind: u32,
    minimum: usize,
) -> Result<(), ParseError> {
    if record.len() < minimum {
        return Err(ParseError::KnownRecordTooShort {
            offset,
            kind,
            minimum,
            actual: record.len(),
        });
    }

    Ok(())
}

fn occupies_loopback(internet_pcb: InternetPcb) -> bool {
    let ipv4_address = Ipv4Addr::from([
        internet_pcb.local_address[INPCB_IPV4_ADDRESS_OFFSET - INPCB_LOCAL_ADDRESS_OFFSET],
        internet_pcb.local_address[INPCB_IPV4_ADDRESS_OFFSET - INPCB_LOCAL_ADDRESS_OFFSET + 1],
        internet_pcb.local_address[INPCB_IPV4_ADDRESS_OFFSET - INPCB_LOCAL_ADDRESS_OFFSET + 2],
        internet_pcb.local_address[INPCB_IPV4_ADDRESS_OFFSET - INPCB_LOCAL_ADDRESS_OFFSET + 3],
    ]);
    let ipv4_occupies_loopback = internet_pcb.version_flags & INP_IPV4 != 0
        && (ipv4_address.is_loopback() || ipv4_address.is_unspecified());

    let ipv6_address = Ipv6Addr::from(internet_pcb.local_address);
    let ipv6_occupies_loopback = internet_pcb.version_flags & INP_IPV6 != 0
        && (ipv6_address.is_loopback() || ipv6_address.is_unspecified());

    ipv4_occupies_loopback || ipv6_occupies_loopback
}

fn read_native_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_ne_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_native_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_ne_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn read_network_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};
    use std::io;
    use std::net::{Ipv4Addr, Ipv6Addr, TcpListener};

    use anyhow::Result;
    use insta::assert_debug_snapshot;

    use crate::command::run_system_command_output;

    use super::{
        FetchError, INP_IPV4, INP_IPV6, MAX_ATTEMPTS, TCPS_LISTEN, XINPCB_MINIMUM_LENGTH,
        XSO_INPCB, XSO_TCPCB, XTCPCB_MINIMUM_LENGTH, fetch_tcp_table_with, parse_tcp_table,
    };

    const SNAPSHOT_COUNT: u32 = 8;
    const SNAPSHOT_GENERATION: u64 = 100;
    const SOCKET_GENERATION: u64 = 200;

    #[test]
    fn pcb_fixture_covers_address_families_states_generations_and_unknown_records() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.push_listener(
            INP_IPV4,
            IpFixture::V4(Ipv4Addr::LOCALHOST),
            45_000,
            SNAPSHOT_GENERATION,
            TCPS_LISTEN,
        );
        fixture.push_listener(
            INP_IPV4,
            IpFixture::V4(Ipv4Addr::UNSPECIFIED),
            45_001,
            SNAPSHOT_GENERATION,
            TCPS_LISTEN,
        );
        fixture.push_listener(
            INP_IPV6,
            IpFixture::V6(Ipv6Addr::LOCALHOST),
            45_002,
            SNAPSHOT_GENERATION,
            TCPS_LISTEN,
        );
        fixture.push_listener(
            INP_IPV6,
            IpFixture::V6(Ipv6Addr::UNSPECIFIED),
            45_003,
            SNAPSHOT_GENERATION,
            TCPS_LISTEN,
        );
        fixture.push_listener(
            INP_IPV4,
            IpFixture::V4(Ipv4Addr::new(192, 168, 1, 5)),
            45_004,
            SNAPSHOT_GENERATION,
            TCPS_LISTEN,
        );
        fixture.push_listener(
            INP_IPV4,
            IpFixture::V4(Ipv4Addr::LOCALHOST),
            45_005,
            SNAPSHOT_GENERATION,
            4,
        );
        fixture.push_listener(
            INP_IPV6,
            IpFixture::V6(Ipv6Addr::LOCALHOST),
            45_006,
            SNAPSHOT_GENERATION + 1,
            TCPS_LISTEN,
        );
        fixture.push_unknown_record();

        assert_debug_snapshot!(parse_tcp_table(&fixture.finish())?);

        Ok(())
    }

    #[test]
    fn empty_pcb_fixture_matches_xnu_single_envelope_shape() {
        let fixtures = [
            (
                "zero count",
                parse_tcp_table(&envelope(0, SNAPSHOT_GENERATION, SOCKET_GENERATION)),
            ),
            (
                "nonzero count",
                parse_tcp_table(&envelope(
                    SNAPSHOT_COUNT,
                    SNAPSHOT_GENERATION,
                    SOCKET_GENERATION,
                )),
            ),
        ];

        assert_debug_snapshot!(fixtures);
    }

    #[test]
    fn malformed_pcb_fixtures_return_deterministic_typed_errors() {
        let mut invalid_envelope = Fixture::new().finish();
        invalid_envelope[0..4].copy_from_slice(&16_u32.to_ne_bytes());

        let mut invalid_record_length = Fixture::new().finish();
        invalid_record_length.splice(24..24, record(16, 0x400));

        let mut truncated_record = Fixture::new().finish();
        truncated_record.splice(24..24, record(32, 0x400));
        truncated_record[24..28].copy_from_slice(&1_024_u32.to_ne_bytes());

        let mut incomplete_record_fixture = Fixture::new();
        incomplete_record_fixture.push_internet_pcb(
            INP_IPV4,
            IpFixture::V4(Ipv4Addr::LOCALHOST),
            45_000,
            SNAPSHOT_GENERATION,
        );

        let mut changed_snapshot = Fixture::new().finish();
        let trailer_offset = changed_snapshot.len() - 24;
        changed_snapshot[trailer_offset + 8..trailer_offset + 16]
            .copy_from_slice(&(SNAPSHOT_GENERATION + 1).to_ne_bytes());

        let fixtures = [
            ("empty", Vec::new()),
            ("invalid envelope", invalid_envelope),
            ("invalid record length", invalid_record_length),
            ("truncated record", truncated_record),
            ("incomplete record", incomplete_record_fixture.finish()),
            ("changed snapshot", changed_snapshot),
        ];
        let errors = fixtures
            .into_iter()
            .map(|(name, fixture)| (name, parse_tcp_table(&fixture)))
            .collect::<Vec<_>>();

        assert_debug_snapshot!(errors);
    }

    #[test]
    fn pcb_fetch_retries_table_growth_and_uses_reported_length() -> Result<()> {
        let mut steps = VecDeque::from([
            QueryStep::Size(4),
            QueryStep::Growth,
            QueryStep::Size(8),
            QueryStep::Data(vec![1, 2, 3]),
        ]);
        let table = fetch_tcp_table_with(|buffer| {
            let Some(step) = steps.pop_front() else {
                return Err(io::Error::other("unexpected query"));
            };

            match (step, buffer) {
                (QueryStep::Size(size), None) => Ok(size),
                (QueryStep::Growth, Some(_)) => Err(io::Error::from_raw_os_error(libc::ENOMEM)),
                (QueryStep::Data(data), Some(buffer)) => {
                    buffer[..data.len()].copy_from_slice(&data);
                    Ok(data.len())
                }
                _ => Err(io::Error::other("query shape did not match fixture")),
            }
        })?;

        assert_eq!(table, [1, 2, 3]);
        assert!(steps.is_empty());

        Ok(())
    }

    #[test]
    fn pcb_fetch_bounds_repeated_table_growth() {
        let mut calls = 0;
        let result = fetch_tcp_table_with(|buffer| {
            calls += 1;
            if buffer.is_some() {
                Err(io::Error::from_raw_os_error(libc::ENOMEM))
            } else {
                Ok(4)
            }
        });

        assert!(matches!(
            result,
            Err(FetchError::TableGrowth { attempts }) if attempts == MAX_ATTEMPTS
        ));
        assert_eq!(calls, MAX_ATTEMPTS * 2);
    }

    #[test]
    #[ignore = "acceptance-only: depends on the live macOS TCP PCB table"]
    fn live_kernel_table_repeatedly_detects_all_controlled_listener_classes() -> Result<()> {
        let ipv4_loopback = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let ipv4_wildcard = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        let ipv6_loopback = TcpListener::bind((Ipv6Addr::LOCALHOST, 0))?;
        let ipv6_wildcard = TcpListener::bind((Ipv6Addr::UNSPECIFIED, 0))?;
        let expected = [
            ("ipv4 loopback", ipv4_loopback.local_addr()?.port()),
            ("ipv4 wildcard", ipv4_wildcard.local_addr()?.port()),
            ("ipv6 loopback", ipv6_loopback.local_addr()?.port()),
            ("ipv6 wildcard", ipv6_wildcard.local_addr()?.port()),
        ];
        let mut detections = expected.map(|(name, _port)| (name, 0, 0));

        for _sample in 0..10 {
            let kernel_ports = crate::loopback_tcp_listener_ports()?;
            let netstat_output =
                run_system_command_output("/usr/sbin/netstat", &["-anv", "-p", "tcp"])?;
            let netstat_ports = controlled_netstat_listener_ports(&netstat_output, &expected);

            for (index, (_name, port)) in expected.iter().enumerate() {
                if kernel_ports.contains(port) {
                    detections[index].1 += 1;
                }
                if netstat_ports.contains(port) {
                    detections[index].2 += 1;
                }
            }
        }

        assert_debug_snapshot!(detections);

        Ok(())
    }

    fn controlled_netstat_listener_ports(output: &str, expected: &[(&str, u16)]) -> BTreeSet<u16> {
        output
            .lines()
            .filter_map(|line| {
                let columns = line.split_whitespace().collect::<Vec<_>>();
                let [
                    protocol,
                    _recv_queue,
                    _send_queue,
                    local_address,
                    _foreign_address,
                    state,
                    ..,
                ] = columns.as_slice()
                else {
                    return None;
                };
                if !protocol.starts_with("tcp") || *state != "LISTEN" {
                    return None;
                }

                let (_address, port) = local_address.rsplit_once('.')?;
                let port = port.parse::<u16>().ok()?;

                expected
                    .iter()
                    .any(|(_name, expected_port)| *expected_port == port)
                    .then_some(port)
            })
            .collect()
    }

    #[derive(Debug)]
    enum QueryStep {
        Size(usize),
        Data(Vec<u8>),
        Growth,
    }

    enum IpFixture {
        V4(Ipv4Addr),
        V6(Ipv6Addr),
    }

    struct Fixture {
        bytes: Vec<u8>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                bytes: envelope(SNAPSHOT_COUNT, SNAPSHOT_GENERATION, SOCKET_GENERATION),
            }
        }

        fn push_listener(
            &mut self,
            version_flags: u8,
            address: IpFixture,
            port: u16,
            generation: u64,
            state: u32,
        ) {
            self.push_internet_pcb(version_flags, address, port, generation);

            let mut tcp_record = record(XTCPCB_MINIMUM_LENGTH, XSO_TCPCB);
            tcp_record[36..40].copy_from_slice(&state.to_ne_bytes());
            self.bytes.extend(tcp_record);
        }

        fn push_internet_pcb(
            &mut self,
            version_flags: u8,
            address: IpFixture,
            port: u16,
            generation: u64,
        ) {
            let mut internet_record = record(XINPCB_MINIMUM_LENGTH, XSO_INPCB);
            internet_record[18..20].copy_from_slice(&port.to_be_bytes());
            internet_record[28..36].copy_from_slice(&generation.to_ne_bytes());
            internet_record[44] = version_flags;
            match address {
                IpFixture::V4(address) => {
                    internet_record[76..80].copy_from_slice(&address.octets());
                }
                IpFixture::V6(address) => {
                    internet_record[64..80].copy_from_slice(&address.octets());
                }
            }
            self.bytes.extend(internet_record);
        }

        fn push_unknown_record(&mut self) {
            self.bytes.extend(record(32, 0x400));
        }

        fn finish(mut self) -> Vec<u8> {
            self.bytes.extend(envelope(
                SNAPSHOT_COUNT,
                SNAPSHOT_GENERATION,
                SOCKET_GENERATION,
            ));
            self.bytes
        }
    }

    fn envelope(count: u32, generation: u64, socket_generation: u64) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(24);
        bytes.extend(24_u32.to_ne_bytes());
        bytes.extend(count.to_ne_bytes());
        bytes.extend(generation.to_ne_bytes());
        bytes.extend(socket_generation.to_ne_bytes());
        bytes
    }

    fn record(length: usize, kind: u32) -> Vec<u8> {
        let mut bytes = vec![0; length];
        bytes[0..4].copy_from_slice(&(length as u32).to_ne_bytes());
        bytes[4..8].copy_from_slice(&kind.to_ne_bytes());
        bytes
    }
}
