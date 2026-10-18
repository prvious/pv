use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::thread;
use std::time::Duration;

use async_io::Timer;
use camino::{Utf8Path, Utf8PathBuf};
use futures_util::{Stream, StreamExt};
use rtnetlink::packet_core::{
    NLM_F_DUMP, NLM_F_DUMP_FILTERED, NLM_F_DUMP_INTR, NLM_F_REQUEST, NetlinkMessage, NetlinkPayload,
};
use rtnetlink::packet_route::AddressFamily as NetlinkAddressFamily;
use rtnetlink::packet_route::RouteNetlinkMessage;
use rtnetlink::packet_route::address::{
    AddressAttribute, AddressFlags, AddressHeaderFlags, AddressMessage, AddressProtocol,
    AddressScope,
};
use rtnetlink::packet_route::link::{InfoKind, LinkAttribute, LinkFlags, LinkInfo, LinkMessage};
use rtnetlink::packet_route::route::{
    RouteAddress, RouteAttribute, RouteMessage, RouteProtocol, RouteScope, RouteType,
};
use rtnetlink::sys::AsyncSocket;
use rtnetlink::{Handle, LinkDummy, LinkUnspec};
use rustix::io::Errno;
use rustix::net::AddressFamily;
use rustix::process::{geteuid, getuid};
use tokio::runtime::Builder;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Proxy, connection, proxy};

use super::{
    DNS_LISTEN_ADDRESS, PV_MARKER, ResolverConfig, ResolverFileState, SYSTEM_RESOLVER_TEST_PATH,
};
use crate::PlatformError;
use crate::helper::{linux as root_files, validate_high_port};
use crate::systemd::{
    native_proxy, read_owned_unit, require_no_candidate, system_manager_proxy, unit_job, within,
    write_owned_unit,
};

const LINK_NAME: &str = "pv0";
const LINK_ALIAS: &str = "Managed by PV";
const UNIT_NAME: &str = "pv-dns.service";
const DNS_SCOPE_MASK: u64 = 1 << 0;
pub(crate) const DNS_INSPECTION_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const DNS_MUTATION_TIMEOUT: Duration = Duration::from_secs(45);
const UNIT_PREFIX: &str = "# Managed by PV\n\
[Unit]\n\
Description=PV .test DNS routing\n\
BindsTo=systemd-resolved.service\n\
After=systemd-resolved.service\n\
\n\
[Service]\n\
Type=oneshot\n\
RemainAfterExit=yes\n\
ExecStart=/usr/local/libexec/pv/pv-helper dns-link up --port ";
const UNIT_SUFFIX: &str = "\n\
ExecStop=/usr/local/libexec/pv/pv-helper dns-link down\n\
\n\
[Install]\n\
WantedBy=systemd-resolved.service\n";

fn render_unit(config: &ResolverConfig) -> String {
    format!("{UNIT_PREFIX}{}{UNIT_SUFFIX}", config.port)
}

fn parse_unit(content: &str) -> Option<ResolverConfig> {
    let port = content
        .strip_prefix(UNIT_PREFIX)?
        .strip_suffix(UNIT_SUFFIX)?
        .parse::<u16>()
        .ok()?;
    let config = ResolverConfig::new(port);
    (port >= 1024 && content == render_unit(&config)).then_some(config)
}

#[derive(Clone)]
struct UnitStatus {
    active_state: String,
    unit_file_state: String,
    fragment_path: Utf8PathBuf,
    drop_in_paths: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LinkSettings {
    dns: Vec<(i32, Vec<u8>, u16, String)>,
    domains: Vec<(String, bool)>,
    default_route: bool,
    dnssec: String,
    dns_over_tls: String,
    llmnr: String,
    multicast_dns: String,
}

impl LinkSettings {
    fn for_config(config: &ResolverConfig) -> Self {
        Self {
            dns: vec![(
                i32::from(AddressFamily::INET.as_raw()),
                DNS_LISTEN_ADDRESS.octets().to_vec(),
                config.port,
                String::new(),
            )],
            domains: vec![("test".to_owned(), true)],
            default_route: false,
            dnssec: "no".to_owned(),
            dns_over_tls: "no".to_owned(),
            llmnr: "no".to_owned(),
            multicast_dns: "no".to_owned(),
        }
    }
}

#[derive(Clone, Default)]
struct Observation {
    unit_content: Option<String>,
    unit_status: Option<UnitStatus>,
    links: Vec<LinkMessage>,
    addresses: Vec<AddressMessage>,
    routes: Vec<RouteMessage>,
    // Strict kernel filtering also associates routes with unexpanded nexthop IDs.
    associated_routes: Vec<RouteMessage>,
    routing_domains: Vec<(i32, String, bool)>,
    link_settings: Option<LinkSettings>,
    scopes_mask: u64,
}

impl Observation {
    fn require_owned(&self) -> Result<(), PlatformError> {
        if self.ownership_conflict() {
            Err(conflict())
        } else {
            Ok(())
        }
    }

    fn require_apply(&self) -> Result<(), PlatformError> {
        self.require_owned()?;
        if self.apply_conflict() {
            Err(conflict())
        } else {
            Ok(())
        }
    }

    fn ownership_conflict(&self) -> bool {
        let path = Utf8PathBuf::from(SYSTEM_RESOLVER_TEST_PATH);
        let link = pv_link(&self.links);
        self.unit_content
            .as_ref()
            .is_some_and(|content| !content.lines().any(|line| line.trim() == PV_MARKER))
            || self.unit_status.as_ref().is_some_and(|status| {
                self.unit_content.is_none()
                    || status.fragment_path != path
                    || !status.drop_in_paths.is_empty()
            })
            || link.is_some_and(|link| !owned_link(link))
            || link.is_some_and(|link| {
                let index = link.header.index;
                self.addresses.iter().any(|address| {
                    address.header.index == index
                        && !dns_address(address)
                        && automatic_link_local(address).is_none()
                }) || self
                    .routes
                    .iter()
                    .filter(|route| route_uses_link(route, index))
                    .chain(&self.associated_routes)
                    .any(|route| {
                        !dns_route(route, index)
                            && !automatic_link_local_route(route, index, &self.addresses)
                    })
            })
    }

    fn apply_conflict(&self) -> bool {
        let index = pv_link(&self.links).map(|link| link.header.index);
        let resolved_index = index.and_then(|index| i32::try_from(index).ok());
        self
                .routing_domains
                .iter()
                .any(|(index, domain, _route_only)| {
                    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
                    (domain == "test" || domain.ends_with(".test")) && Some(*index) != resolved_index
                })
            || self.addresses.iter().any(|address| {
                Some(address.header.index) != index
                    && address.attributes.iter().any(|attribute| {
                        matches!(attribute, AddressAttribute::Address(IpAddr::V4(address))
                            | AddressAttribute::Local(IpAddr::V4(address)) if *address == DNS_LISTEN_ADDRESS)
                    })
            })
            || self.routes.iter().any(|route| {
                endpoint_route(route)
                    && !index.is_some_and(|index| route_uses_link(route, index))
                    && !self.links.iter().any(|link| {
                        link.header.flags.contains(LinkFlags::Loopback)
                            && loopback_route(route, link.header.index)
                    })
            })
    }

    fn owned_missing(&self) -> bool {
        self.unit_content.is_none() && self.unit_status.is_none() && pv_link(&self.links).is_none()
    }

    fn link_current(&self, config: &ResolverConfig) -> bool {
        pv_link(&self.links).is_some_and(|link| {
            link.header.flags.contains(LinkFlags::Up)
                && self.addresses.iter().any(|address| {
                    address.header.index == link.header.index && current_dns_address(address)
                })
                && self.routes.iter().any(|route| {
                    dns_route(route, link.header.index)
                        && route.header.scope == RouteScope::Host
                        && route.attributes.contains(&RouteAttribute::PrefSource(
                            RouteAddress::Inet(DNS_LISTEN_ADDRESS),
                        ))
                })
        }) && self.link_settings.as_ref() == Some(&LinkSettings::for_config(config))
            && self.scopes_mask & DNS_SCOPE_MASK != 0
    }

    fn state(&self, expected: Option<&ResolverConfig>) -> ResolverFileState {
        let path = Utf8PathBuf::from(SYSTEM_RESOLVER_TEST_PATH);
        if self.require_apply().is_err() {
            return ResolverFileState::Conflict { path };
        }

        if self.owned_missing() {
            return ResolverFileState::Missing { path };
        }

        let actual = self.unit_content.as_deref().and_then(parse_unit);
        if let Some(actual) = &actual
            && expected.is_none_or(|expected| expected == actual)
            && self.unit_status.as_ref().is_some_and(|status| {
                status.active_state == "active" && status.unit_file_state == "enabled"
            })
            && self.link_current(actual)
        {
            ResolverFileState::Current {
                path,
                port: actual.port,
            }
        } else {
            ResolverFileState::Stale {
                path,
                expected_port: expected.map(|config| config.port),
                actual_port: actual.map(|config| config.port),
            }
        }
    }
}

fn pv_link(links: &[LinkMessage]) -> Option<&LinkMessage> {
    links.iter().find(|link| {
        link.attributes
            .iter()
            .any(|attribute| matches!(attribute, LinkAttribute::IfName(name) if name == LINK_NAME))
    })
}

fn owned_link(link: &LinkMessage) -> bool {
    link.attributes
        .iter()
        .any(|attribute| matches!(attribute, LinkAttribute::IfAlias(alias) if alias == LINK_ALIAS))
        && dummy_link(link)
        && !link
            .attributes
            .iter()
            .any(|attribute| matches!(attribute, LinkAttribute::Controller(index) if *index != 0))
}

fn dns_address(address: &AddressMessage) -> bool {
    let expected = IpAddr::V4(DNS_LISTEN_ADDRESS);
    address.header.family == NetlinkAddressFamily::Inet
        && address.header.prefix_len == 32
        && address
            .attributes
            .contains(&AddressAttribute::Address(expected))
        && address
            .attributes
            .contains(&AddressAttribute::Local(expected))
        && address.attributes.iter().all(|attribute| match attribute {
            AddressAttribute::Address(value) | AddressAttribute::Local(value) => *value == expected,
            AddressAttribute::Label(label) => label == LINK_NAME,
            AddressAttribute::CacheInfo(_)
            | AddressAttribute::Flags(_)
            | AddressAttribute::Protocol(_)
            | AddressAttribute::RoutePriority(_) => true,
            _ => false,
        })
}

fn current_dns_address(address: &AddressMessage) -> bool {
    dns_address(address)
        && address.header.scope == AddressScope::Universe
        && address.header.flags.contains(AddressHeaderFlags::Permanent)
        && address.attributes.iter().all(|attribute| {
            !matches!(attribute, AddressAttribute::Flags(flags) if !flags.contains(AddressFlags::Permanent))
        })
        && address.attributes.iter().any(|attribute| {
            matches!(attribute, AddressAttribute::CacheInfo(info)
                if info.ifa_valid == u32::MAX && info.ifa_preferred == u32::MAX)
        })
}

fn automatic_link_local(address: &AddressMessage) -> Option<Ipv6Addr> {
    if address.header.family != NetlinkAddressFamily::Inet6
        || address.header.prefix_len != 64
        || address.header.scope != AddressScope::Link
    {
        return None;
    }
    let value = address
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            AddressAttribute::Address(IpAddr::V6(value)) if value.is_unicast_link_local() => {
                Some(*value)
            }
            _ => None,
        })?;
    address
        .attributes
        .iter()
        .all(|attribute| match attribute {
            AddressAttribute::Address(address) | AddressAttribute::Local(address) => {
                *address == IpAddr::V6(value)
            }
            AddressAttribute::Label(label) => label == LINK_NAME,
            AddressAttribute::Protocol(protocol) => *protocol == AddressProtocol::LinkLocal,
            AddressAttribute::CacheInfo(_) | AddressAttribute::Flags(_) => true,
            _ => false,
        })
        .then_some(value)
}

fn route_uses_link(route: &RouteMessage, index: u32) -> bool {
    route.attributes.iter().any(|attribute| match attribute {
        RouteAttribute::Oif(output) => *output == index,
        RouteAttribute::MultiPath(hops) => hops.iter().any(|hop| hop.interface_index == index),
        _ => false,
    })
}

fn route_table(route: &RouteMessage) -> u32 {
    route
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            RouteAttribute::Table(table) => Some(*table),
            _ => None,
        })
        .unwrap_or(u32::from(route.header.table))
}

fn direct_kernel_route(route: &RouteMessage, index: u32) -> bool {
    route.header.protocol == RouteProtocol::Kernel
        && route.header.source_prefix_length == 0
        && route.header.tos == 0
        && route.attributes.contains(&RouteAttribute::Oif(index))
        && route.attributes.iter().all(|attribute| match attribute {
            RouteAttribute::Oif(output) => *output == index,
            RouteAttribute::Table(table) => *table == route_table(route),
            RouteAttribute::Destination(_)
            | RouteAttribute::PrefSource(_)
            | RouteAttribute::Priority(_)
            | RouteAttribute::CacheInfo(_)
            | RouteAttribute::Preference(_) => true,
            _ => false,
        })
}

fn dns_route(route: &RouteMessage, index: u32) -> bool {
    route.header.address_family == NetlinkAddressFamily::Inet
        && route.header.destination_prefix_length == 32
        && route.header.kind == RouteType::Local
        && route_table(route) == 255
        && direct_kernel_route(route, index)
        && route
            .attributes
            .contains(&RouteAttribute::Destination(RouteAddress::Inet(
                DNS_LISTEN_ADDRESS,
            )))
}

fn automatic_link_local_route(
    route: &RouteMessage,
    index: u32,
    addresses: &[AddressMessage],
) -> bool {
    if route.header.address_family != NetlinkAddressFamily::Inet6
        || !direct_kernel_route(route, index)
    {
        return false;
    }
    let destination = route
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            RouteAttribute::Destination(RouteAddress::Inet6(address)) => Some(*address),
            _ => None,
        });
    let Some(destination) = destination else {
        return false;
    };
    match (
        route.header.kind,
        route_table(route),
        route.header.destination_prefix_length,
    ) {
        (RouteType::Local, 255, 128) => addresses.iter().any(|address| {
            address.header.index == index && automatic_link_local(address) == Some(destination)
        }),
        (RouteType::Unicast, 254, 64) => {
            destination == Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0)
                && addresses.iter().any(|address| {
                    address.header.index == index && automatic_link_local(address).is_some()
                })
        }
        (RouteType::Multicast, 255, 8) => destination == Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0),
        _ => false,
    }
}

fn endpoint_route(route: &RouteMessage) -> bool {
    let prefix = route.header.destination_prefix_length;
    if route.header.address_family != NetlinkAddressFamily::Inet || !(8..=32).contains(&prefix) {
        return false;
    }
    let mask = u32::MAX << (32 - prefix);
    route.attributes.iter().any(|attribute| {
        matches!(attribute, RouteAttribute::Destination(RouteAddress::Inet(address))
            if u32::from(*address) & mask == u32::from(DNS_LISTEN_ADDRESS) & mask)
    })
}

fn loopback_route(route: &RouteMessage, index: u32) -> bool {
    route.header.destination_prefix_length == 8
        && route.header.kind == RouteType::Local
        && route_table(route) == 255
        && direct_kernel_route(route, index)
        && route
            .attributes
            .contains(&RouteAttribute::Destination(RouteAddress::Inet(
                Ipv4Addr::new(127, 0, 0, 0),
            )))
        && route
            .attributes
            .contains(&RouteAttribute::PrefSource(RouteAddress::Inet(
                Ipv4Addr::LOCALHOST,
            )))
}

fn dummy_link(link: &LinkMessage) -> bool {
    link.attributes.iter().any(|attribute| {
        matches!(attribute, LinkAttribute::LinkInfo(info)
            if info.contains(&LinkInfo::Kind(InfoKind::Dummy)))
    })
}

pub(super) fn inspect(expected: Option<&ResolverConfig>) -> ResolverFileState {
    match run_native("inspection", DNS_INSPECTION_TIMEOUT, async {
        let (_, observation) = observe(false).await?;
        Ok(observation)
    }) {
        Ok(observation) => observation.state(expected),
        Err(error) => ResolverFileState::Unreadable {
            path: SYSTEM_RESOLVER_TEST_PATH.into(),
            message: error.to_string(),
        },
    }
}

fn run_native<T: Send>(
    operation: &'static str,
    timeout: Duration,
    future: impl Future<Output = Result<T, PlatformError>> + Send,
) -> Result<T, PlatformError> {
    thread::scope(|scope| {
        thread::Builder::new()
            .name("pv-resolver-native".to_owned())
            .spawn_scoped(scope, move || {
                // The caller may already be inside a Tokio runtime. Keep this runtime
                // and its local futures on one thread; dropping them does not cancel
                // work already accepted by systemd.
                let runtime = Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(PlatformError::ResolverIo)?;
                runtime.block_on(async {
                    within(timeout, future).await.ok_or_else(|| {
                        PlatformError::SystemIntegration(format!(
                            "resolver {operation} timed out after {} seconds; inspect DNS state before retrying",
                            timeout.as_secs(),
                        ))
                    })?
                })
            })
            .map_err(PlatformError::ResolverIo)?
            .join()
            .map_err(|_| {
                PlatformError::SystemIntegration(format!("resolver {operation} thread failed"))
            })?
    })
}

async fn query_network(handle: &Handle) -> Result<Observation, PlatformError> {
    let links = query_links(handle).await?;
    let index = pv_link(&links).map(|link| link.header.index);
    let addresses = read_dump(
        handle,
        RouteNetlinkMessage::GetAddress(AddressMessage::default()),
        false,
    )
    .await?
    .into_iter()
    .map(|message| match message {
        NetlinkMessage {
            payload: NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewAddress(address)),
            ..
        } => Ok(address),
        message => Err(netlink_error(rtnetlink::Error::UnexpectedMessage(message))),
    })
    .collect::<Result<_, _>>()?;
    let mut routes = Vec::new();
    let mut associated_routes = Vec::new();
    for family in [NetlinkAddressFamily::Inet, NetlinkAddressFamily::Inet6] {
        routes.extend(query_routes(handle, family, None).await?);
        if let Some(index) = index {
            associated_routes.extend(query_routes(handle, family, Some(index)).await?);
        }
    }
    Ok(Observation {
        links,
        addresses,
        routes,
        associated_routes,
        ..Observation::default()
    })
}

async fn query_links(handle: &Handle) -> Result<Vec<LinkMessage>, PlatformError> {
    read_dump(
        handle,
        RouteNetlinkMessage::GetLink(LinkMessage::default()),
        false,
    )
    .await?
    .into_iter()
    .map(|message| match message {
        NetlinkMessage {
            payload: NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link)),
            ..
        } => Ok(link),
        message => Err(netlink_error(rtnetlink::Error::UnexpectedMessage(message))),
    })
    .collect()
}

async fn query_routes(
    handle: &Handle,
    family: NetlinkAddressFamily,
    index: Option<u32>,
) -> Result<Vec<RouteMessage>, PlatformError> {
    let mut request = RouteMessage::default();
    request.header.address_family = family;
    if let Some(index) = index {
        request.attributes.push(RouteAttribute::Oif(index));
    }
    read_dump(
        handle,
        RouteNetlinkMessage::GetRoute(request),
        index.is_some(),
    )
    .await?
    .into_iter()
    .map(|message| match message {
        NetlinkMessage {
            payload: NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewRoute(route)),
            ..
        } if route.header.address_family == family => Ok(route),
        message => Err(netlink_error(rtnetlink::Error::UnexpectedMessage(message))),
    })
    .collect()
}

async fn read_dump(
    handle: &Handle,
    request: RouteNetlinkMessage,
    filtered: bool,
) -> Result<Vec<NetlinkMessage<RouteNetlinkMessage>>, PlatformError> {
    let mut request = NetlinkMessage::from(request);
    request.header.flags = NLM_F_REQUEST | NLM_F_DUMP;
    let mut handle = handle.clone();
    let responses = handle.request(request).map_err(netlink_error)?;
    collect_dump(responses, filtered).await
}

async fn collect_dump(
    mut responses: impl Stream<Item = NetlinkMessage<RouteNetlinkMessage>> + Unpin,
    filtered: bool,
) -> Result<Vec<NetlinkMessage<RouteNetlinkMessage>>, PlatformError> {
    let mut messages = Vec::new();
    let mut completed = false;
    while let Some(response) = responses.next().await {
        if let NetlinkPayload::Error(error) = &response.payload {
            return Err(netlink_error(rtnetlink::Error::NetlinkError(error.clone())));
        }
        if let NetlinkPayload::Done(done) = &response.payload
            && done.code != 0
        {
            if let Some(errno) = done.code.checked_neg().filter(|errno| *errno > 0) {
                return Err(PlatformError::ResolverIo(io::Error::from_raw_os_error(
                    errno,
                )));
            }
            return Err(PlatformError::SystemIntegration(format!(
                "resolver network inventory failed with status {}; inspect DNS state before retrying",
                done.code,
            )));
        }
        if response.header.flags & NLM_F_DUMP_INTR != 0 {
            return Err(PlatformError::SystemIntegration(
                "resolver network inventory changed during inspection; retry before changing DNS state".to_owned(),
            ));
        }
        if filtered && response.header.flags & NLM_F_DUMP_FILTERED == 0 {
            return Err(PlatformError::SystemIntegration(
                "resolver network inventory did not confirm its interface filter".to_owned(),
            ));
        }
        match &response.payload {
            NetlinkPayload::InnerMessage(_) if !completed => messages.push(response),
            NetlinkPayload::Done(_) if !completed => completed = true,
            _ => return Err(netlink_error(rtnetlink::Error::UnexpectedMessage(response))),
        }
    }
    if completed {
        Ok(messages)
    } else {
        Err(PlatformError::SystemIntegration(
            "resolver network inventory ended without a successful completion".to_owned(),
        ))
    }
}

async fn with_netlink<T, F: Future<Output = Result<T, PlatformError>>>(
    operation: impl FnOnce(Handle) -> F,
) -> Result<T, PlatformError> {
    let (mut connection, handle, _) =
        rtnetlink::new_connection().map_err(PlatformError::ResolverIo)?;
    // Convenience dump streams discard interruption flags and terminal errors.
    connection.set_forward_done(true);
    connection
        .socket_mut()
        .socket_mut()
        .set_netlink_get_strict_chk(true)
        .map_err(PlatformError::ResolverIo)?;
    // Keep the original handle alive until the request has finished, so the driver
    // cannot finish normally merely because the operation dropped its own clone.
    let request = operation(handle.clone());
    tokio::select! {
        biased;
        // A failed driver closes response channels, which could otherwise look like
        // a successful empty/partial dump. Poll it before accepting stream completion.
        () = connection => Err(PlatformError::SystemIntegration(
            "resolver netlink connection ended before the operation completed".to_owned(),
        )),
        result = request => result,
    }
}

async fn observe(
    allow_resolved_activation: bool,
) -> Result<(Connection, Observation), PlatformError> {
    let (connection, mut observation) = observe_owned().await?;
    let resolved = resolved_manager(&connection, allow_resolved_activation).await?;
    observation.routing_domains = resolved.get_property("Domains").await.map_err(bus_error)?;
    if let Some(link) = pv_link(&observation.links) {
        let index = i32::try_from(link.header.index)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
        if let Some((settings, scopes_mask)) = read_link_settings(&resolved, index).await? {
            observation.link_settings = Some(settings);
            observation.scopes_mask = scopes_mask;
        }
    }
    Ok((connection, observation))
}

async fn observe_owned() -> Result<(Connection, Observation), PlatformError> {
    let unit_content = root_files::read_root_file(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH), 0o644)?
        .map(String::from_utf8)
        .transpose()
        .map_err(|error| {
            PlatformError::ResolverIo(io::Error::new(io::ErrorKind::InvalidData, error))
        })?;
    let mut observation =
        with_netlink(|handle| async move { query_network(&handle).await }).await?;
    // Use the real system bus for both privileged and unprivileged callers.
    let connection = connection::Builder::address("unix:path=/run/dbus/system_bus_socket")
        .map_err(bus_error)?
        .build()
        .await
        .map_err(bus_error)?;
    let systemd = native_proxy(
        &connection,
        OwnedObjectPath::try_from("/org/freedesktop/systemd1")
            .map_err(zbus::Error::from)
            .map_err(bus_error)?,
        "org.freedesktop.systemd1.Manager",
    )
    .await
    .map_err(bus_error)?;
    let unit_status = read_unit_status(&systemd, unit_content.is_some()).await?;
    observation.unit_content = unit_content;
    observation.unit_status = unit_status;
    Ok((connection, observation))
}

async fn resolved_manager(
    connection: &Connection,
    allow_activation: bool,
) -> Result<Proxy<'static>, PlatformError> {
    let name = BusName::try_from("org.freedesktop.resolve1")
        .map_err(zbus::Error::from)
        .map_err(bus_error)?;
    let destination = if allow_activation {
        name
    } else {
        // A unique destination cannot activate a stopped service or follow a new owner.
        DBusProxy::new(connection)
            .await
            .map_err(bus_error)?
            .get_name_owner(name)
            .await
            .map_err(zbus::Error::from)
            .map_err(bus_error)?
            .into()
    };
    resolved_proxy(
        connection,
        destination,
        "/org/freedesktop/resolve1"
            .try_into()
            .map_err(zbus::Error::from)
            .map_err(bus_error)?,
        "org.freedesktop.resolve1.Manager",
    )
    .await
}

async fn read_unit_status(
    manager: &Proxy<'_>,
    has_unit_file: bool,
) -> Result<Option<UnitStatus>, PlatformError> {
    let path: OwnedObjectPath = match manager.call("LoadUnit", &(UNIT_NAME,)).await {
        Ok(path) => path,
        Err(error) if method_error(&error, "org.freedesktop.systemd1.NoSuchUnit") => {
            return Ok(None);
        }
        Err(error) => return Err(bus_error(error)),
    };
    let unit = native_proxy(manager.connection(), path, "org.freedesktop.systemd1.Unit")
        .await
        .map_err(bus_error)?;
    let load_state: String = unit.get_property("LoadState").await.map_err(bus_error)?;
    let fragment_path: String = unit.get_property("FragmentPath").await.map_err(bus_error)?;
    let active_state: String = unit.get_property("ActiveState").await.map_err(bus_error)?;
    if load_state == "not-found" && fragment_path.is_empty() && active_state == "inactive" {
        return Ok(None);
    }
    Ok(Some(UnitStatus {
        active_state,
        unit_file_state: if has_unit_file {
            manager
                .call("GetUnitFileState", &(UNIT_NAME,))
                .await
                .map_err(bus_error)?
        } else {
            String::new()
        },
        fragment_path: fragment_path.into(),
        drop_in_paths: unit.get_property("DropInPaths").await.map_err(bus_error)?,
    }))
}

async fn resolved_proxy(
    connection: &Connection,
    destination: BusName<'static>,
    path: OwnedObjectPath,
    interface: &'static str,
) -> Result<Proxy<'static>, PlatformError> {
    proxy::Builder::new(connection)
        .destination(destination)
        .map_err(bus_error)?
        .path(path)
        .map_err(bus_error)?
        .interface(interface)
        .map_err(bus_error)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .map_err(bus_error)
}

async fn read_link_settings(
    manager: &Proxy<'_>,
    index: i32,
) -> Result<Option<(LinkSettings, u64)>, PlatformError> {
    let Some(path) = resolved_link_path(manager, index).await? else {
        return Ok(None);
    };
    let link = resolved_proxy(
        manager.connection(),
        manager.destination().to_owned(),
        path,
        "org.freedesktop.resolve1.Link",
    )
    .await?;
    let scopes_mask = link.get_property("ScopesMask").await.map_err(bus_error)?;
    Ok(Some((
        LinkSettings {
            dns: link.get_property("DNSEx").await.map_err(bus_error)?,
            domains: link.get_property("Domains").await.map_err(bus_error)?,
            default_route: link.get_property("DefaultRoute").await.map_err(bus_error)?,
            dnssec: link.get_property("DNSSEC").await.map_err(bus_error)?,
            dns_over_tls: link.get_property("DNSOverTLS").await.map_err(bus_error)?,
            llmnr: link.get_property("LLMNR").await.map_err(bus_error)?,
            multicast_dns: link.get_property("MulticastDNS").await.map_err(bus_error)?,
        },
        scopes_mask,
    )))
}

async fn resolved_link_path(
    manager: &Proxy<'_>,
    index: i32,
) -> Result<Option<OwnedObjectPath>, PlatformError> {
    match manager.call("GetLink", &(index,)).await {
        Ok(path) => Ok(Some(path)),
        Err(error) if method_error(&error, "org.freedesktop.resolve1.NoSuchLink") => Ok(None),
        Err(error) => Err(bus_error(error)),
    }
}

/// Configures the fixed PV dummy link and its per-link resolved routing. Requires root.
pub fn start_resolver_link(port: u16) -> Result<(), PlatformError> {
    require_root()?;
    validate_high_port("DNS", port)?;
    run_native("link up", Duration::from_secs(10), async {
        let (connection, before) = observe(true).await?;
        before.require_apply()?;
        // A failed ExecStart does not run ExecStop. Recreate retained owned state
        // because replacing an address does not repair its scope or missing route.
        stop_link().await?;
        let resolved = resolved_manager(&connection, true).await?;
        with_netlink(|handle| async move {
            let before = query_network(&handle).await?;
            before.require_apply()?;
            if pv_link(&before.links).is_some() {
                return Err(conflict());
            }
            handle
                .link()
                .add(LinkDummy::new(LINK_NAME).build())
                .execute()
                .await
                .map_err(netlink_error)?;
            // The kernel applies aliases through SETLINK, not link creation.
            let created_links = query_links(&handle).await?;
            let created = pv_link(&created_links).ok_or_else(|| {
                PlatformError::SystemIntegration(
                    "PV dummy link is missing after creation".to_owned(),
                )
            })?;
            if !dummy_link(created)
                || created.attributes.iter().any(|attribute| {
                    matches!(attribute, LinkAttribute::IfAlias(alias) if !alias.is_empty())
                })
            {
                return Err(conflict());
            }
            let index = created.header.index;
            handle
                .link()
                .set(LinkUnspec::new_with_index(index).alias(LINK_ALIAS).build())
                .execute()
                .await
                .map_err(netlink_error)?;
            let owned = query_network(&handle).await?;
            owned.require_apply()?;
            if owned_pv_link(&owned.links)?.is_none_or(|link| link.header.index != index) {
                return Err(conflict());
            }
            let mut address = handle.address().add(index, DNS_LISTEN_ADDRESS.into(), 32);
            address.message_mut().header.scope = AddressScope::Universe;
            address.execute().await.map_err(netlink_error)?;
            handle
                .link()
                .set(LinkUnspec::new_with_index(index).up().build())
                .execute()
                .await
                .map_err(netlink_error)?;
            let resolved_index = i32::try_from(index)
                .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
            // resolved handles kernel notifications independently of netlink ACKs.
            while resolved_link_path(&resolved, resolved_index)
                .await?
                .is_none()
            {
                Timer::after(Duration::from_millis(25)).await;
            }
            let config = ResolverConfig::new(port);
            let settings = LinkSettings::for_config(&config);
            resolved
                .call::<_, _, ()>(
                    "SetLinkDefaultRoute",
                    &(resolved_index, settings.default_route),
                )
                .await
                .map_err(bus_error)?;
            for (method, mode) in [
                ("SetLinkDNSSEC", &settings.dnssec),
                ("SetLinkDNSOverTLS", &settings.dns_over_tls),
                ("SetLinkLLMNR", &settings.llmnr),
                ("SetLinkMulticastDNS", &settings.multicast_dns),
            ] {
                resolved
                    .call::<_, _, ()>(method, &(resolved_index, mode))
                    .await
                    .map_err(bus_error)?;
            }
            resolved
                .call::<_, _, ()>("SetLinkDNSEx", &(resolved_index, &settings.dns))
                .await
                .map_err(bus_error)?;
            resolved
                .call::<_, _, ()>("SetLinkDomains", &(resolved_index, &settings.domains))
                .await
                .map_err(bus_error)?;
            loop {
                let mut after = query_network(&handle).await?;
                after.routing_domains =
                    resolved.get_property("Domains").await.map_err(bus_error)?;
                after.require_apply()?;
                if owned_pv_link(&after.links)?.is_none_or(|link| link.header.index != index) {
                    return Err(conflict());
                }
                if let Some((actual, scopes_mask)) =
                    read_link_settings(&resolved, resolved_index).await?
                {
                    after.link_settings = Some(actual);
                    after.scopes_mask = scopes_mask;
                }
                // ExecStart cannot require its own unit to have become active yet.
                if after.link_current(&config) {
                    return Ok(());
                }
                Timer::after(Duration::from_millis(25)).await;
            }
        })
        .await
    })
}

/// Reverts resolved settings when available and deletes only the fixed PV-owned dummy link.
/// Requires root.
pub fn stop_resolver_link() -> Result<(), PlatformError> {
    require_root()?;
    run_native("link down", Duration::from_secs(10), stop_link())
}

async fn stop_link() -> Result<(), PlatformError> {
    let (connection, before) = observe_owned().await?;
    before.require_owned()?;
    with_netlink(|handle| async move {
        let current = query_network(&handle).await?;
        current.require_owned()?;
        let Some(link) = owned_pv_link(&current.links)? else {
            return Ok(());
        };
        let index = link.header.index;
        let resolved_index = i32::try_from(index)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
        let bus = DBusProxy::new(&connection).await.map_err(bus_error)?;
        if bus
            .name_has_owner(
                "org.freedesktop.resolve1"
                    .try_into()
                    .map_err(zbus::Error::from)
                    .map_err(bus_error)?,
            )
            .await
            .map_err(zbus::Error::from)
            .map_err(bus_error)?
        {
            let resolved = resolved_manager(&connection, false).await?;
            match resolved
                .call::<_, _, ()>("RevertLink", &(resolved_index,))
                .await
            {
                Ok(()) => {}
                Err(error) if method_error(&error, "org.freedesktop.resolve1.NoSuchLink") => {}
                Err(error) => return Err(bus_error(error)),
            }
        }
        let current = query_network(&handle).await?;
        current.require_owned()?;
        if let Some(link) = owned_pv_link(&current.links)? {
            if link.header.index != index {
                return Err(conflict());
            }
            handle
                .link()
                .del(index)
                .execute()
                .await
                .map_err(netlink_error)?;
        }
        let after = query_network(&handle).await?;
        if pv_link(&after.links).is_some()
            || after.links.iter().any(|link| link.header.index == index)
            || after
                .addresses
                .iter()
                .any(|address| address.header.index == index)
            || after
                .routes
                .iter()
                .any(|route| route_uses_link(route, index))
        {
            return Err(PlatformError::SystemIntegration(
                "pv0 is still present after removal; inspect the link before retrying".to_owned(),
            ));
        }
        // Strict interface filtering reports ENODEV for a deleted index rather
        // than a successful empty dump. Other errors cannot establish absence.
        match query_routes(&handle, NetlinkAddressFamily::Inet, Some(index)).await {
            Err(PlatformError::ResolverIo(error))
                if error.raw_os_error() == Some(Errno::NODEV.raw_os_error()) => {}
            Err(PlatformError::ResolverNetlink(error))
                if matches!(error.as_ref(), rtnetlink::Error::NetlinkError(message)
                    if message.code.is_some_and(|code| code.get() == -Errno::NODEV.raw_os_error())) => {}
            Err(error) => return Err(error),
            Ok(_) => return Err(PlatformError::SystemIntegration(
                "removed DNS interface index still exists; inspect the link before retrying".to_owned(),
            )),
        }
        Ok(())
    })
    .await
}

pub(crate) fn apply_resolver_config_privileged(
    config: &ResolverConfig,
) -> Result<(), PlatformError> {
    require_root()?;
    validate_high_port("DNS", config.port)?;
    run_native("apply", DNS_MUTATION_TIMEOUT, async {
        root_files::validate_directory_chain(Utf8Path::new("/usr/local/libexec"), false)?;
        let _lock = root_files::acquire_lock(Utf8Path::new(root_files::LOCK_PATH))?;
        let path = Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH);
        root_files::validate_directory_chain(Utf8Path::new("/etc/systemd/system"), false)?;
        read_owned_unit(path)?;
        require_no_candidate(path)?;
        let (connection, before) = observe(true).await?;
        before.require_apply()?;
        let manager = system_manager_proxy(&connection).await.map_err(bus_error)?;
        write_unit(path, config)?;
        manager
            .call::<_, _, ()>("Reload", &())
            .await
            .map_err(bus_error)?;
        // Reload may select a newly visible fragment or drop-in. Check the manager's
        // actual selection before enablement or execution.
        let (_, reloaded) = observe(true).await?;
        reloaded.require_apply()?;
        manager
            .call::<_, _, (bool, Vec<(String, String, String)>)>(
                "EnableUnitFiles",
                &([UNIT_NAME].as_slice(), false, false),
            )
            .await
            .map_err(bus_error)?;
        manager
            .call::<_, _, ()>("Reload", &())
            .await
            .map_err(bus_error)?;
        let (_, enabled) = observe(true).await?;
        enabled.require_apply()?;
        dns_unit_job(&manager, "RestartUnit").await?;
        let (_, after) = observe(true).await?;
        match after.state(Some(config)) {
            ResolverFileState::Current { .. } => Ok(()),
            state => Err(PlatformError::SystemIntegration(format!(
                "DNS state did not match after apply: {state:?}"
            ))),
        }
    })
    .map_err(|source| PlatformError::ResolverMutation {
        operation: "apply",
        source: Box::new(source),
    })
}

pub(crate) fn remove_resolver_config_privileged() -> Result<(), PlatformError> {
    require_root()?;
    run_native("remove", DNS_MUTATION_TIMEOUT, async {
        root_files::validate_directory_chain(Utf8Path::new("/usr/local/libexec"), false)?;
        let _lock = root_files::acquire_lock(Utf8Path::new(root_files::LOCK_PATH))?;
        let path = Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH);
        root_files::validate_directory_chain(Utf8Path::new("/etc/systemd/system"), false)?;
        let content = read_owned_unit(path)?;
        require_no_candidate(path)?;
        let (connection, before) = observe_owned().await?;
        before.require_owned()?;
        let manager = system_manager_proxy(&connection).await.map_err(bus_error)?;
        for runtime in [false, true] {
            match manager
                .call::<_, _, Vec<(String, String, String)>>(
                    "DisableUnitFiles",
                    &([UNIT_NAME].as_slice(), runtime),
                )
                .await
            {
                Ok(_) => {}
                Err(error)
                    if content.is_none()
                        && method_error(&error, "org.freedesktop.systemd1.NoSuchUnit") => {}
                Err(error) => return Err(bus_error(error)),
            }
        }
        manager
            .call::<_, _, ()>("Reload", &())
            .await
            .map_err(bus_error)?;
        let (_, reloaded) = observe_owned().await?;
        reloaded.require_owned()?;
        dns_unit_job(&manager, "StopUnit").await?;
        match manager
            .call::<_, _, ()>("ResetFailedUnit", &(UNIT_NAME,))
            .await
        {
            Ok(()) => {}
            Err(error) if method_error(&error, "org.freedesktop.systemd1.NoSuchUnit") => {}
            Err(error) => return Err(bus_error(error)),
        }
        // Stopping an inactive or missing unit does not execute ExecStop. Clean up
        // a retained owned link through the same native path as dns-link down.
        stop_link().await?;
        read_owned_unit(path)?;
        state::fs::remove_file_if_exists(path)
            .and_then(|()| state::fs::sync_parent_directory(path))
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
        manager
            .call::<_, _, ()>("Reload", &())
            .await
            .map_err(bus_error)?;
        let (_, after) = observe_owned().await?;
        if after.owned_missing() {
            Ok(())
        } else {
            Err(PlatformError::SystemIntegration(format!(
                "DNS owned resources were not absent after removal: {:?}",
                after.state(None)
            )))
        }
    })
    .map_err(|source| PlatformError::ResolverMutation {
        operation: "remove",
        source: Box::new(source),
    })
}

async fn dns_unit_job(manager: &Proxy<'_>, operation: &str) -> Result<(), PlatformError> {
    let result = within(
        Duration::from_secs(25),
        unit_job(manager, operation, UNIT_NAME),
    )
    .await
    .ok_or_else(|| {
        PlatformError::SystemIntegration(format!("systemd {operation} {UNIT_NAME} timed out"))
    })?
    .map_err(bus_error)?;
    if result == "done" {
        Ok(())
    } else {
        Err(PlatformError::SystemIntegration(format!(
            "systemd {operation} {UNIT_NAME} finished with {result}"
        )))
    }
}

fn write_unit(path: &Utf8Path, config: &ResolverConfig) -> Result<(), PlatformError> {
    write_owned_unit(path, &render_unit(config))
}

fn require_root() -> Result<(), PlatformError> {
    if getuid().is_root() && geteuid().is_root() {
        Ok(())
    } else {
        Err(PlatformError::PrivilegedHelperAuthentication(
            "DNS link operations require root".to_owned(),
        ))
    }
}

fn owned_pv_link(links: &[LinkMessage]) -> Result<Option<&LinkMessage>, PlatformError> {
    let link = pv_link(links);
    if link.is_some_and(|link| !owned_link(link)) {
        Err(conflict())
    } else {
        Ok(link)
    }
}

fn conflict() -> PlatformError {
    PlatformError::SystemIntegration(format!(
        "Linux DNS configuration conflicts with administrator configuration; inspect {SYSTEM_RESOLVER_TEST_PATH}, its systemd overrides and pv0 ownership, addresses and routes. Before applying routing, repair competing test search or routing domains, including subdomains. For a retained unit cache, run `sudo systemctl daemon-reload` before retrying",
    ))
}

fn netlink_error(error: rtnetlink::Error) -> PlatformError {
    PlatformError::ResolverNetlink(Box::new(error))
}

fn method_error(error: &zbus::Error, expected: &str) -> bool {
    matches!(error, zbus::Error::MethodError(name, _, _) if name.as_str() == expected)
}

fn bus_error(error: zbus::Error) -> PlatformError {
    PlatformError::ResolverBus(Box::new(error))
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroI32;

    use crate::systemd::read_unit;
    use anyhow::Result;
    use camino_tempfile::tempdir;
    use futures_util::stream;
    use insta::assert_snapshot;
    use rtnetlink::packet_core::{DoneMessage, ErrorMessage, NetlinkHeader};
    use rtnetlink::packet_route::address::CacheInfo;
    use rtnetlink::packet_route::route::RouteNextHop;
    use rtnetlink::{AddressMessageBuilder, LinkDummy, RouteMessageBuilder};
    use rustix::fs::{CWD, Mode, mkfifoat};

    use super::*;

    fn test_address(index: u32) -> AddressMessage {
        let mut address = AddressMessageBuilder::<Ipv4Addr>::new()
            .index(index)
            .address(Ipv4Addr::new(127, 0, 0, 2), 32)
            .build();
        address.header.scope = AddressScope::Universe;
        address.header.flags = AddressHeaderFlags::Permanent;
        let mut lifetime = CacheInfo::default();
        lifetime.ifa_valid = u32::MAX;
        lifetime.ifa_preferred = u32::MAX;
        address
            .attributes
            .push(AddressAttribute::CacheInfo(lifetime));
        address
    }

    fn test_route(index: u32) -> RouteMessage {
        RouteMessageBuilder::<Ipv4Addr>::new()
            .destination_prefix(Ipv4Addr::new(127, 0, 0, 2), 32)
            .pref_source(Ipv4Addr::new(127, 0, 0, 2))
            .output_interface(index)
            .table_id(255)
            .protocol(RouteProtocol::Kernel)
            .scope(RouteScope::Host)
            .kind(RouteType::Local)
            .build()
    }

    fn current() -> Observation {
        let config = ResolverConfig::new(35353);
        Observation {
            unit_content: Some(render_unit(&config)),
            unit_status: Some(UnitStatus {
                active_state: "active".to_owned(),
                unit_file_state: "enabled".to_owned(),
                fragment_path: SYSTEM_RESOLVER_TEST_PATH.into(),
                drop_in_paths: Vec::new(),
            }),
            links: vec![
                LinkDummy::new(LINK_NAME)
                    .index(7)
                    .up()
                    .append_extra_attribute(LinkAttribute::IfAlias(LINK_ALIAS.to_owned()))
                    .build(),
            ],
            addresses: vec![test_address(7)],
            routes: vec![test_route(7)],
            associated_routes: vec![test_route(7)],
            routing_domains: vec![(7, "test".to_owned(), true)],
            link_settings: Some(LinkSettings {
                dns: vec![(2, vec![127, 0, 0, 2], 35353, String::new())],
                domains: vec![("test".to_owned(), true)],
                default_route: false,
                dnssec: "no".to_owned(),
                dns_over_tls: "no".to_owned(),
                llmnr: "no".to_owned(),
                multicast_dns: "no".to_owned(),
            }),
            scopes_mask: 1,
        }
    }

    #[test]
    fn generated_unit_is_fixed_except_for_a_high_port() {
        let config = ResolverConfig::new(35353);
        let rendered = render_unit(&config);
        assert_snapshot!(rendered, @r"
        # Managed by PV
        [Unit]
        Description=PV .test DNS routing
        BindsTo=systemd-resolved.service
        After=systemd-resolved.service

        [Service]
        Type=oneshot
        RemainAfterExit=yes
        ExecStart=/usr/local/libexec/pv/pv-helper dns-link up --port 35353
        ExecStop=/usr/local/libexec/pv/pv-helper dns-link down

        [Install]
        WantedBy=systemd-resolved.service
        ");
        for port in [1024, 35353, u16::MAX] {
            let config = ResolverConfig::new(port);
            assert_eq!(parse_unit(&render_unit(&config)), Some(config));
        }
        for content in [
            render_unit(&ResolverConfig::new(53)),
            rendered.replace("35353", "65536"),
            rendered.replace("35353", "+35353"),
            rendered.replace("/usr/local/libexec/pv/pv-helper", "/tmp/pv-helper"),
            format!("{rendered}ExecStartPost=/tmp/extra-command\n"),
        ] {
            assert_eq!(parse_unit(&content), None);
        }
    }

    #[test]
    fn current_requires_the_active_unit_owned_up_link_and_exact_resolved_settings() {
        let config = ResolverConfig::new(35353);
        let path = Utf8PathBuf::from(SYSTEM_RESOLVER_TEST_PATH);
        let observation = current();
        assert!(Observation::default().require_owned().is_ok());
        assert!(observation.require_owned().is_ok());
        assert_eq!(
            Observation::default().state(Some(&config)),
            ResolverFileState::Missing { path: path.clone() }
        );
        for expected in [None, Some(&config)] {
            assert_eq!(
                observation.state(expected),
                ResolverFileState::Current {
                    path: path.clone(),
                    port: config.port
                }
            );
        }
        assert_eq!(
            observation.state(Some(&ResolverConfig::new(45000))),
            ResolverFileState::Stale {
                path: path.clone(),
                expected_port: Some(45000),
                actual_port: Some(config.port)
            }
        );

        let mut stale = Vec::new();
        let mut inactive = observation.clone();
        if let Some(status) = &mut inactive.unit_status {
            status.active_state = "inactive".to_owned();
        }
        stale.push(("inactive unit", inactive));
        for unit_file_state in ["disabled", "enabled-runtime"] {
            let mut disabled = observation.clone();
            if let Some(status) = &mut disabled.unit_status {
                status.unit_file_state = unit_file_state.to_owned();
            }
            stale.push((unit_file_state, disabled));
        }
        let mut missing_link = observation.clone();
        missing_link.links.clear();
        missing_link.addresses.clear();
        missing_link.routes.clear();
        missing_link.associated_routes.clear();
        missing_link.routing_domains.clear();
        stale.push(("missing link", missing_link));
        let mut down_link = observation.clone();
        down_link.links[0].header.flags.remove(LinkFlags::Up);
        stale.push(("down link", down_link));
        let mut missing_settings = observation.clone();
        missing_settings.link_settings = None;
        stale.push(("unconfigured resolved link", missing_settings));
        let mut wrong_port = observation.clone();
        if let Some(settings) = &mut wrong_port.link_settings {
            settings.dns[0].2 = 45000;
        }
        stale.push(("wrong DNS port", wrong_port));
        let mut wrong_address = observation.clone();
        if let Some(settings) = &mut wrong_address.link_settings {
            settings.dns[0].1 = vec![10, 0, 0, 1];
        }
        stale.push(("foreign DNS server", wrong_address));
        let mut default_route = observation.clone();
        if let Some(settings) = &mut default_route.link_settings {
            settings.default_route = true;
        }
        stale.push(("upstream default route", default_route));
        let mut search_domain = observation.clone();
        if let Some(settings) = &mut search_domain.link_settings {
            settings.domains[0].1 = false;
        }
        stale.push(("search domain instead of routing domain", search_domain));
        let mut dnssec = observation.clone();
        if let Some(settings) = &mut dnssec.link_settings {
            settings.dnssec = "allow-downgrade".to_owned();
        }
        stale.push(("inherited DNSSEC", dnssec));
        let mut malformed_unit = observation.clone();
        malformed_unit.unit_content = Some(format!("{PV_MARKER}\n[Service]\n"));
        assert_eq!(
            malformed_unit.state(Some(&config)),
            ResolverFileState::Stale {
                path: path.clone(),
                expected_port: Some(config.port),
                actual_port: None
            }
        );
        let mut retained_link = observation.clone();
        retained_link.unit_content = None;
        retained_link.unit_status = None;
        assert_eq!(
            retained_link.state(Some(&config)),
            ResolverFileState::Stale {
                path: path.clone(),
                expected_port: Some(config.port),
                actual_port: None
            }
        );
        for (case, observation) in stale {
            assert!(observation.require_owned().is_ok(), "{case}");
            assert_eq!(
                observation.state(Some(&config)),
                ResolverFileState::Stale {
                    path: path.clone(),
                    expected_port: Some(config.port),
                    actual_port: Some(config.port)
                },
                "{case}"
            );
        }
    }

    #[test]
    fn foreign_networking_or_manager_selected_units_are_conflicts() {
        let mut conflicts = Vec::new();
        let mut unmarked = current();
        unmarked.unit_content = Some(
            render_unit(&ResolverConfig::new(35353)).replace(PV_MARKER, "# administrator unit"),
        );
        conflicts.push(("unmarked unit", unmarked));
        let mut foreign_fragment = current();
        if let Some(status) = &mut foreign_fragment.unit_status {
            status.fragment_path = "/run/systemd/system/pv-dns.service".into();
        }
        conflicts.push(("manager selected another unit", foreign_fragment));
        let mut missing_loaded_file = current();
        missing_loaded_file.unit_content = None;
        conflicts.push((
            "loaded unit with missing ownership file",
            missing_loaded_file,
        ));
        let mut drop_in = current();
        if let Some(status) = &mut drop_in.unit_status {
            status
                .drop_in_paths
                .push("/etc/systemd/system/pv-dns.service.d/override.conf".to_owned());
        }
        conflicts.push(("administrator drop-in", drop_in));
        let mut unmarked_link = current();
        unmarked_link.links[0]
            .attributes
            .retain(|attribute| !matches!(attribute, LinkAttribute::IfAlias(_)));
        conflicts.push(("unmarked pv0", unmarked_link));
        let mut wrong_alias = current();
        wrong_alias.links[0]
            .attributes
            .retain(|attribute| !matches!(attribute, LinkAttribute::IfAlias(_)));
        wrong_alias.links[0]
            .attributes
            .push(LinkAttribute::IfAlias("administrator".to_owned()));
        conflicts.push(("foreign alias", wrong_alias));
        let mut wrong_kind = current();
        wrong_kind.links[0]
            .attributes
            .retain(|attribute| !matches!(attribute, LinkAttribute::LinkInfo(_)));
        wrong_kind.links[0]
            .attributes
            .push(LinkAttribute::LinkInfo(vec![LinkInfo::Kind(
                InfoKind::Veth,
            )]));
        conflicts.push(("same alias on non-dummy", wrong_kind));
        for (case, observation) in &conflicts {
            assert!(observation.require_owned().is_err(), "{case}");
        }
        for index in [0, 9] {
            for route_only in [false, true] {
                for domain in ["TeSt.", "app.Acme.TeSt."] {
                    let mut other_route = Observation::default();
                    other_route
                        .routing_domains
                        .push((index, domain.to_owned(), route_only));
                    conflicts.push(("foreign test domain without PV installation", other_route));
                }
            }
        }
        let mut concurrent_route = current();
        concurrent_route
            .routing_domains
            .push((9, "test".to_owned(), true));
        conflicts.push((
            "foreign routing domain on configured system",
            concurrent_route,
        ));
        for (case, observation) in conflicts {
            assert!(observation.require_apply().is_err(), "{case}");
            assert_eq!(
                observation.state(None),
                ResolverFileState::Conflict {
                    path: SYSTEM_RESOLVER_TEST_PATH.into()
                },
                "{case}"
            );
        }
        for domain in ["contest", "test.example", "."] {
            let unrelated = Observation {
                routing_domains: vec![(9, domain.to_owned(), false)],
                ..Observation::default()
            };
            assert_eq!(
                unrelated.state(None),
                ResolverFileState::Missing {
                    path: SYSTEM_RESOLVER_TEST_PATH.into()
                }
            );
        }
    }

    #[test]
    fn native_dns_readiness_and_ownership_have_distinct_outcomes() {
        let mut cases = vec![("current", current())];
        let mut addressless = current();
        addressless.addresses.clear();
        addressless.routes.clear();
        cases.push(("addressless", addressless));
        let mut missing_route = current();
        missing_route.routes.clear();
        cases.push(("missing route", missing_route));
        for mask in [0, 2] {
            let mut missing_scope = current();
            missing_scope.scopes_mask = mask;
            cases.push((
                if mask == 0 {
                    "no effective scope"
                } else {
                    "only non-DNS scope"
                },
                missing_scope,
            ));
        }
        let mut wrong_scope = current();
        wrong_scope.addresses[0].header.scope = AddressScope::Host;
        cases.push(("host address scope", wrong_scope));
        let mut finite_lifetime = current();
        finite_lifetime.addresses[0]
            .header
            .flags
            .remove(AddressHeaderFlags::Permanent);
        for attribute in &mut finite_lifetime.addresses[0].attributes {
            if let AddressAttribute::CacheInfo(info) = attribute {
                info.ifa_valid = 60;
                info.ifa_preferred = 60;
            }
        }
        cases.push(("finite address lifetime", finite_lifetime));
        let mut route_scope = current();
        route_scope.routes[0].header.scope = RouteScope::Link;
        cases.push(("kernel local route scope drift", route_scope));
        let mut preferred_source = current();
        for attribute in &mut preferred_source.routes[0].attributes {
            if let RouteAttribute::PrefSource(address) = attribute {
                *address = RouteAddress::Inet(Ipv4Addr::LOCALHOST);
            }
        }
        cases.push(("kernel local preferred source drift", preferred_source));
        let mut wrong_prefix = current();
        wrong_prefix.addresses[0].header.prefix_len = 8;
        cases.push(("wrong address prefix", wrong_prefix));
        let mut peer = current();
        for attribute in &mut peer.addresses[0].attributes {
            if let AddressAttribute::Address(address) = attribute {
                *address = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
            }
        }
        cases.push(("distinct address peer", peer));
        let mut foreign_address = current();
        foreign_address.addresses.push(
            AddressMessageBuilder::<Ipv4Addr>::new()
                .index(7)
                .address(Ipv4Addr::new(192, 0, 2, 1), 32)
                .build(),
        );
        cases.push(("foreign static address on pv0", foreign_address));
        let mut foreign_route = current();
        foreign_route.routes.push(
            RouteMessageBuilder::<Ipv4Addr>::new()
                .destination_prefix(Ipv4Addr::new(192, 0, 2, 0), 24)
                .output_interface(7)
                .build(),
        );
        cases.push(("foreign static route on pv0", foreign_route));
        let mut multipath = current();
        let mut hop = RouteNextHop::default();
        hop.interface_index = 7;
        multipath.routes.push(
            RouteMessageBuilder::<Ipv4Addr>::new()
                .destination_prefix(Ipv4Addr::new(192, 0, 2, 0), 24)
                .multipath(vec![hop])
                .build(),
        );
        cases.push(("foreign multipath route on pv0", multipath));
        let mut associated_nexthop = current();
        let mut route = RouteMessageBuilder::<Ipv4Addr>::new()
            .destination_prefix(Ipv4Addr::new(192, 0, 2, 0), 24)
            .build();
        route.attributes.push(RouteAttribute::NhId(17));
        associated_nexthop.routes.push(route.clone());
        associated_nexthop.associated_routes.push(route.clone());
        cases.push(("kernel-filtered nexthop on pv0", associated_nexthop));
        let mut unrelated_nexthop = current();
        unrelated_nexthop.routes.push(route);
        cases.push(("nexthop on unrelated link", unrelated_nexthop));
        let mut controller = current();
        controller.links[0]
            .attributes
            .push(LinkAttribute::Controller(9));
        cases.push(("attached controller", controller));
        let mut external_address = current();
        external_address.addresses.push(test_address(9));
        cases.push(("external explicit endpoint", external_address));
        let mut external_route = current();
        let mut route = test_route(9);
        route.header.protocol = RouteProtocol::Static;
        external_route.routes.push(route);
        cases.push(("external endpoint route", external_route));
        let mut foreign_domain = current();
        foreign_domain
            .routing_domains
            .push((9, "test".to_owned(), true));
        cases.push(("external routing domain", foreign_domain));
        let mut foreign_protocol = current();
        foreign_protocol.routes[0].header.protocol = RouteProtocol::Static;
        cases.push(("foreign protocol on endpoint route", foreign_protocol));
        let mut gateway = current();
        gateway.routes[0]
            .attributes
            .push(RouteAttribute::Gateway(RouteAddress::Inet(
                Ipv4Addr::LOCALHOST,
            )));
        cases.push(("forwarding authority on endpoint route", gateway));
        let mut ordinary_loopback = current();
        let mut loopback = LinkUnspec::new_with_index(1).name("lo").build();
        loopback.header.flags.insert(LinkFlags::Loopback);
        ordinary_loopback.links.push(loopback);
        ordinary_loopback.addresses.push(
            AddressMessageBuilder::<Ipv4Addr>::new()
                .index(1)
                .address(Ipv4Addr::LOCALHOST, 8)
                .build(),
        );
        ordinary_loopback.routes.push(
            RouteMessageBuilder::<Ipv4Addr>::new()
                .destination_prefix(Ipv4Addr::new(127, 0, 0, 0), 8)
                .pref_source(Ipv4Addr::LOCALHOST)
                .output_interface(1)
                .table_id(255)
                .protocol(RouteProtocol::Kernel)
                .kind(RouteType::Local)
                .scope(RouteScope::Host)
                .build(),
        );
        cases.push(("ordinary loopback covering route", ordinary_loopback));
        let mut link_local = current();
        let address = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 7);
        let mut native_address = AddressMessageBuilder::<Ipv6Addr>::new()
            .index(7)
            .address(address, 64)
            .build();
        native_address.header.scope = AddressScope::Link;
        native_address
            .attributes
            .push(AddressAttribute::Protocol(AddressProtocol::LinkLocal));
        link_local.addresses.push(native_address);
        for (destination, prefix, table, kind) in [
            (address, 128, 255, RouteType::Local),
            (
                Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0),
                64,
                254,
                RouteType::Unicast,
            ),
            (
                Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0),
                8,
                255,
                RouteType::Multicast,
            ),
        ] {
            link_local.routes.push(
                RouteMessageBuilder::<Ipv6Addr>::new()
                    .destination_prefix(destination, prefix)
                    .output_interface(7)
                    .table_id(table)
                    .protocol(RouteProtocol::Kernel)
                    .kind(kind)
                    .build(),
            );
        }
        cases.push(("automatic IPv6 link-local state", link_local.clone()));
        let mut foreign_protocol = link_local.clone();
        for attribute in &mut foreign_protocol.addresses[1].attributes {
            if let AddressAttribute::Protocol(protocol) = attribute {
                *protocol = AddressProtocol::Other(99);
            }
        }
        cases.push(("foreign IPv6 address protocol", foreign_protocol));
        link_local.routes.push(
            RouteMessageBuilder::<Ipv6Addr>::new()
                .destination_prefix(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 8), 128)
                .output_interface(7)
                .table_id(255)
                .protocol(RouteProtocol::Kernel)
                .kind(RouteType::Local)
                .build(),
        );
        cases.push(("unrelated kernel IPv6 local route", link_local));
        let results = cases
            .into_iter()
            .map(|(name, observation)| {
                let state = match observation.state(Some(&ResolverConfig::new(35353))) {
                    ResolverFileState::Missing { .. } => "Missing",
                    ResolverFileState::Current { .. } => "Current",
                    ResolverFileState::Stale { .. } => "Stale",
                    ResolverFileState::Conflict { .. } => "Conflict",
                    ResolverFileState::Unreadable { .. } => "Unreadable",
                };
                format!(
                    "{name}: {state}, apply={}, remove={}",
                    observation.require_apply().is_ok(),
                    observation.require_owned().is_ok()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_snapshot!(results, @r"
        current: Current, apply=true, remove=true
        addressless: Stale, apply=true, remove=true
        missing route: Stale, apply=true, remove=true
        no effective scope: Stale, apply=true, remove=true
        only non-DNS scope: Stale, apply=true, remove=true
        host address scope: Stale, apply=true, remove=true
        finite address lifetime: Stale, apply=true, remove=true
        kernel local route scope drift: Stale, apply=true, remove=true
        kernel local preferred source drift: Stale, apply=true, remove=true
        wrong address prefix: Conflict, apply=false, remove=false
        distinct address peer: Conflict, apply=false, remove=false
        foreign static address on pv0: Conflict, apply=false, remove=false
        foreign static route on pv0: Conflict, apply=false, remove=false
        foreign multipath route on pv0: Conflict, apply=false, remove=false
        kernel-filtered nexthop on pv0: Conflict, apply=false, remove=false
        nexthop on unrelated link: Current, apply=true, remove=true
        attached controller: Conflict, apply=false, remove=false
        external explicit endpoint: Conflict, apply=false, remove=true
        external endpoint route: Conflict, apply=false, remove=true
        external routing domain: Conflict, apply=false, remove=true
        foreign protocol on endpoint route: Conflict, apply=false, remove=false
        forwarding authority on endpoint route: Conflict, apply=false, remove=false
        ordinary loopback covering route: Current, apply=true, remove=true
        automatic IPv6 link-local state: Current, apply=true, remove=true
        foreign IPv6 address protocol: Conflict, apply=false, remove=false
        unrelated kernel IPv6 local route: Conflict, apply=false, remove=false
        ");
    }

    #[tokio::test]
    async fn network_dumps_require_consistent_filtered_completion() -> Result<()> {
        let done = |code, flags| {
            let mut message = DoneMessage::default();
            message.code = code;
            let mut header = NetlinkHeader::default();
            header.flags = flags;
            NetlinkMessage::new(header, NetlinkPayload::Done(message))
        };
        let data = NetlinkMessage::from(RouteNetlinkMessage::NewRoute(test_route(7)));
        let mut filtered_data = data.clone();
        filtered_data.header.flags = NLM_F_DUMP_FILTERED;
        for (messages, filtered, expected) in [
            (vec![done(0, 0)], false, vec![]),
            (vec![data.clone(), done(0, 0)], false, vec![data.clone()]),
            (vec![done(0, NLM_F_DUMP_FILTERED)], true, vec![]),
            (
                vec![filtered_data.clone(), done(0, NLM_F_DUMP_FILTERED)],
                true,
                vec![filtered_data.clone()],
            ),
        ] {
            assert_eq!(
                collect_dump(stream::iter(messages), filtered).await?,
                expected
            );
        }
        let mut interrupted_data = data.clone();
        interrupted_data.header.flags = NLM_F_DUMP_INTR;
        for (case, messages, filtered) in [
            (
                "interrupted inner response",
                vec![interrupted_data, done(0, 0)],
                false,
            ),
            (
                "interrupted terminal response",
                vec![data.clone(), done(0, NLM_F_DUMP_INTR)],
                false,
            ),
            ("missing terminal response", vec![data.clone()], false),
            ("unconfirmed empty filter", vec![done(0, 0)], true),
            (
                "unconfirmed terminal filter",
                vec![filtered_data, done(0, 0)],
                true,
            ),
        ] {
            assert!(
                matches!(
                    collect_dump(stream::iter(messages), filtered).await,
                    Err(PlatformError::SystemIntegration(_))
                ),
                "{case}"
            );
        }
        let mut missing_device = ErrorMessage::default();
        missing_device.code = NonZeroI32::new(-Errno::NODEV.raw_os_error());
        let response = NetlinkMessage::new(
            NetlinkHeader::default(),
            NetlinkPayload::Error(missing_device),
        );
        assert!(matches!(collect_dump(stream::iter([response]), true).await,
            Err(PlatformError::ResolverNetlink(error))
                if matches!(error.as_ref(), rtnetlink::Error::NetlinkError(message)
                    if message.code.is_some_and(|code| code.get() == -Errno::NODEV.raw_os_error()))));
        for (errno, filtered, preceding) in [
            (Errno::PERM, false, vec![data]),
            (Errno::NODEV, true, vec![]),
        ] {
            assert!(matches!(
                collect_dump(stream::iter(preceding.into_iter().chain([done(-errno.raw_os_error(), 0)])), filtered).await,
                Err(PlatformError::ResolverIo(error)) if error.raw_os_error() == Some(errno.raw_os_error())
            ));
        }
        Ok(())
    }

    #[test]
    fn unit_reader_refuses_symlinks_and_nonregular_files_without_blocking() -> Result<()> {
        let directory = tempdir()?;
        let unit = directory.path().join(UNIT_NAME);
        let missing = directory.path().join("missing");
        let fifo = directory.path().join("fifo");
        let link = directory.path().join("link");
        let content = render_unit(&ResolverConfig::new(35353));
        state::fs::write_sensitive_file(&unit, &content)?;
        mkfifoat(CWD, fifo.as_std_path(), Mode::RUSR | Mode::WUSR)?;
        state::fs::symlink_file(&unit, &link)?;
        assert_eq!(read_unit(&missing)?, None);
        assert_eq!(read_unit(&unit)?, Some(content));
        for path in [&fifo, &link, directory.path()] {
            assert!(read_unit(path).is_err(), "{path}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn native_link_dump_reads_loopback_without_network_mutations() -> Result<()> {
        let observation = run_native(
            "inspection",
            Duration::from_secs(10),
            with_netlink(|handle| async move { query_network(&handle).await }),
        )?;
        let link = observation
            .links
            .iter()
            .find(|link| {
                link.header.index > 0
            && link.header.flags.contains(LinkFlags::Loopback)
            && link.attributes.iter().any(
                |attribute| matches!(attribute, LinkAttribute::IfName(name) if name == "lo")
            )
            })
            .ok_or_else(|| io::Error::other("native loopback link is missing"))?;
        let index = link.header.index;
        let associated = run_native(
            "inspection",
            Duration::from_secs(10),
            with_netlink(|handle| async move {
                let mut routes =
                    query_routes(&handle, NetlinkAddressFamily::Inet, Some(index)).await?;
                routes
                    .extend(query_routes(&handle, NetlinkAddressFamily::Inet6, Some(index)).await?);
                Ok(routes)
            }),
        )?;
        assert!(observation.addresses.iter().any(|address| {
            address.header.index == link.header.index
                && address.header.prefix_len == 8
                && address
                    .attributes
                    .contains(&AddressAttribute::Local(IpAddr::V4(Ipv4Addr::new(
                        127, 0, 0, 1,
                    ))))
        }));
        for routes in [&observation.routes, &associated] {
            assert!(routes.iter().any(|route| {
                route.header.address_family == NetlinkAddressFamily::Inet
                    && route.header.destination_prefix_length == 8
                    && route.header.kind == RouteType::Local
                    && route.header.protocol == RouteProtocol::Kernel
                    && (route.header.table == 255
                        || route.attributes.contains(&RouteAttribute::Table(255)))
                    && route
                        .attributes
                        .contains(&RouteAttribute::Oif(link.header.index))
                    && route
                        .attributes
                        .contains(&RouteAttribute::Destination(RouteAddress::Inet(
                            Ipv4Addr::new(127, 0, 0, 0),
                        )))
                    && route
                        .attributes
                        .contains(&RouteAttribute::PrefSource(RouteAddress::Inet(
                            Ipv4Addr::LOCALHOST,
                        )))
            }));
        }
        Ok(())
    }

    #[test]
    #[ignore = "run as root in an ordinary disposable Linux container; uses only temporary files"]
    fn root_unit_writes_preserve_foreign_and_retained_files() -> Result<()> {
        require_root()?;
        let directory = camino_tempfile::tempdir_in("/run")?;
        let path = directory.path().join(UNIT_NAME);
        let candidate = Utf8PathBuf::from(format!("{path}.candidate"));
        let initial = ResolverConfig::new(35353);
        let replacement = ResolverConfig::new(45000);
        write_unit(&path, &initial)?;
        assert_eq!(
            read_owned_unit(&path)?,
            Some(render_unit(&initial).into_bytes())
        );
        write_unit(&path, &replacement)?;
        assert_eq!(
            read_owned_unit(&path)?,
            Some(render_unit(&replacement).into_bytes())
        );
        assert_eq!(root_files::read_root_file(&candidate, 0o644)?, None);

        root_files::write_root_file(&candidate, b"retained staged bytes", 0o644)?;
        assert!(write_unit(&path, &initial).is_err());
        assert_eq!(
            read_owned_unit(&path)?,
            Some(render_unit(&replacement).into_bytes())
        );
        assert_eq!(
            root_files::read_root_file(&candidate, 0o644)?,
            Some(b"retained staged bytes".to_vec())
        );

        let foreign_path = directory.path().join("foreign.service");
        root_files::write_root_file(&foreign_path, b"administrator unit", 0o644)?;
        assert!(matches!(
            write_unit(&foreign_path, &initial),
            Err(PlatformError::SystemIntegration(_))
        ));
        assert_eq!(
            root_files::read_root_file(&foreign_path, 0o644)?,
            Some(b"administrator unit".to_vec())
        );
        Ok(())
    }
}
