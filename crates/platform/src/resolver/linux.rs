use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::thread;
use std::time::Duration;

use async_io::Timer;
use camino::{Utf8Path, Utf8PathBuf};
use futures_util::TryStreamExt;
use rtnetlink::packet_route::AddressFamily as NetlinkAddressFamily;
use rtnetlink::packet_route::address::{AddressAttribute, AddressMessage, AddressScope};
use rtnetlink::packet_route::link::{InfoKind, LinkAttribute, LinkFlags, LinkInfo, LinkMessage};
use rtnetlink::{Handle, LinkDummy, LinkUnspec};
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
    }

    fn apply_conflict(&self) -> bool {
        let index = pv_link(&self.links).map(|link| link.header.index);
        let resolved_index = index.and_then(|index| i32::try_from(index).ok());
        self.routing_domains
            .iter()
            .any(|(index, domain, _route_only)| {
                let domain = domain.trim_end_matches('.').to_ascii_lowercase();
                (domain == "test" || domain.ends_with(".test")) && Some(*index) != resolved_index
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
}

fn current_dns_address(address: &AddressMessage) -> bool {
    address.header.family == NetlinkAddressFamily::Inet
        && address.header.prefix_len == 32
        && address.header.scope == AddressScope::Universe
        && address
            .attributes
            .contains(&AddressAttribute::Local(IpAddr::V4(DNS_LISTEN_ADDRESS)))
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
    Ok(Observation {
        links: query_links(handle).await?,
        addresses: handle
            .address()
            .get()
            .execute()
            .try_collect()
            .await
            .map_err(netlink_error)?,
        ..Observation::default()
    })
}

async fn query_links(handle: &Handle) -> Result<Vec<LinkMessage>, PlatformError> {
    handle
        .link()
        .get()
        .execute()
        .try_collect()
        .await
        .map_err(netlink_error)
}

async fn with_netlink<T, F: Future<Output = Result<T, PlatformError>>>(
    operation: impl FnOnce(Handle) -> F,
) -> Result<T, PlatformError> {
    let (connection, handle, _) = rtnetlink::new_connection().map_err(PlatformError::ResolverIo)?;
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
        {
            return Err(PlatformError::SystemIntegration(
                "pv0 is still present after removal; inspect the link before retrying".to_owned(),
            ));
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
        "Linux DNS configuration conflicts with administrator configuration; inspect {SYSTEM_RESOLVER_TEST_PATH}, its systemd overrides and pv0 ownership. Before applying routing, repair competing test search or routing domains, including subdomains. For a retained unit cache, run `sudo systemctl daemon-reload` before retrying",
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
    use std::net::Ipv4Addr;

    use anyhow::Result;
    use camino_tempfile::tempdir;
    use insta::assert_snapshot;
    use rtnetlink::AddressMessageBuilder;
    use rustix::fs::{CWD, Mode, mkfifoat};

    use super::*;
    use crate::systemd::read_unit;

    fn current() -> Observation {
        let mut address = AddressMessageBuilder::<Ipv4Addr>::new()
            .index(7)
            .address(DNS_LISTEN_ADDRESS, 32)
            .build();
        address.header.scope = AddressScope::Universe;
        Observation {
            unit_content: Some(render_unit(&ResolverConfig::new(35353))),
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
            addresses: vec![address],
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
            scopes_mask: DNS_SCOPE_MASK,
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
        assert!(matches!(
            Observation::default().state(Some(&config)),
            ResolverFileState::Missing { .. }
        ));
        assert!(matches!(
            current().state(Some(&config)),
            ResolverFileState::Current { port: 35353, .. }
        ));
        assert!(matches!(
            current().state(Some(&ResolverConfig::new(45000))),
            ResolverFileState::Stale { .. }
        ));
        let mut observation = current();
        observation.addresses[0].header.scope = AddressScope::Link;
        assert!(observation.require_owned().is_ok());
        assert!(matches!(
            observation.state(Some(&config)),
            ResolverFileState::Stale { .. }
        ));
        observation = current();
        if let Some(settings) = &mut observation.link_settings {
            settings.default_route = true;
        }
        assert!(matches!(
            observation.state(Some(&config)),
            ResolverFileState::Stale { .. }
        ));
        observation = current();
        observation.scopes_mask = 0;
        assert!(matches!(
            observation.state(Some(&config)),
            ResolverFileState::Stale { .. }
        ));
        observation = current();
        if let Some(status) = &mut observation.unit_status {
            status.unit_file_state = "disabled".to_owned();
        }
        assert!(matches!(
            observation.state(Some(&config)),
            ResolverFileState::Stale { .. }
        ));
    }

    #[test]
    fn foreign_units_links_and_test_domains_are_conflicts() {
        let mut observation = current();
        observation.unit_content = Some("administrator unit".to_owned());
        assert!(observation.require_owned().is_err());
        observation = current();
        observation.links[0]
            .attributes
            .retain(|attribute| !matches!(attribute, LinkAttribute::IfAlias(_)));
        assert!(observation.require_owned().is_err());
        observation = current();
        observation.links[0]
            .attributes
            .retain(|attribute| !matches!(attribute, LinkAttribute::LinkInfo(_)));
        assert!(observation.require_owned().is_err());
        observation = current();
        if let Some(status) = &mut observation.unit_status {
            status.fragment_path = "/run/foreign.service".into();
        }
        assert!(observation.require_owned().is_err());
        observation = current();
        if let Some(status) = &mut observation.unit_status {
            status
                .drop_in_paths
                .push("/etc/systemd/system/pv-dns.service.d/override.conf".to_owned());
        }
        assert!(observation.require_owned().is_err());
        for (index, domain, route_only, conflict) in [
            (7, "test", true, false),
            (9, "TeSt.", true, true),
            (0, "app.test", false, true),
            (9, "example.tested", false, false),
        ] {
            observation = current();
            observation.routing_domains = vec![(index, domain.to_owned(), route_only)];
            assert_eq!(
                observation.require_apply().is_err(),
                conflict,
                "{index}: {domain}"
            );
        }
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
        let loopback = observation
            .links
            .iter()
            .find(|link| link.header.flags.contains(LinkFlags::Loopback))
            .ok_or_else(|| io::Error::other("native loopback link is missing"))?;
        assert!(observation.addresses.iter().any(|address| {
            address.header.index == loopback.header.index
                && address
                    .attributes
                    .contains(&AddressAttribute::Local(IpAddr::V4(Ipv4Addr::LOCALHOST)))
        }));
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
