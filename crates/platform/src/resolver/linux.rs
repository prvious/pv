use std::future::Future;
use std::io::{self, Read};
use std::thread;
use std::time::Duration;

use async_io::Timer;
use camino::{Utf8Path, Utf8PathBuf};
use futures_util::TryStreamExt;
use rtnetlink::packet_route::link::{InfoKind, LinkAttribute, LinkFlags, LinkInfo, LinkMessage};
use rtnetlink::{Handle, LinkDummy, LinkUnspec};
use rustix::fs::{Mode, OFlags, open};
use rustix::net::AddressFamily;
use rustix::process::{geteuid, getuid};
use tokio::runtime::Builder;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Proxy, connection, proxy};

use super::{PV_MARKER, ResolverConfig, ResolverFileState, SYSTEM_RESOLVER_TEST_PATH};
use crate::PlatformError;
use crate::helper::{linux as root_files, validate_high_port};
use crate::systemd::{native_proxy, system_manager_proxy, unit_job, within};

const LINK_NAME: &str = "pv0";
const LINK_ALIAS: &str = "Managed by PV";
const UNIT_NAME: &str = "pv-dns.service";
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

#[expect(
    clippy::disallowed_types,
    reason = "resolver inspection owns a native NOFOLLOW/NONBLOCK-validated descriptor"
)]
type UnitFile = std::fs::File;

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
                vec![127, 0, 0, 1],
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
    routing_domains: Vec<(i32, String, bool)>,
    link_settings: Option<LinkSettings>,
}

impl Observation {
    fn require_owned(&self) -> Result<(), PlatformError> {
        if matches!(self.state(None), ResolverFileState::Conflict { .. }) {
            Err(conflict())
        } else {
            Ok(())
        }
    }

    fn state(&self, expected: Option<&ResolverConfig>) -> ResolverFileState {
        let path = Utf8PathBuf::from(SYSTEM_RESOLVER_TEST_PATH);
        let link = pv_link(&self.links);
        let link_index = link.and_then(|link| i32::try_from(link.header.index).ok());
        if self
            .unit_content
            .as_ref()
            .is_some_and(|content| !content.lines().any(|line| line.trim() == PV_MARKER))
            || self.unit_status.as_ref().is_some_and(|status| {
                self.unit_content.is_none()
                    || status.fragment_path != path
                    || !status.drop_in_paths.is_empty()
            })
            || link.is_some_and(|link| !owned_link(link))
            || self
                .routing_domains
                .iter()
                .any(|(index, domain, route_only)| {
                    *route_only
                        && domain.trim_end_matches('.').eq_ignore_ascii_case("test")
                        && Some(*index) != link_index
                })
        {
            return ResolverFileState::Conflict { path };
        }

        if self.unit_content.is_none() && self.unit_status.is_none() && link.is_none() {
            return ResolverFileState::Missing { path };
        }

        let actual = self.unit_content.as_deref().and_then(parse_unit);
        if let Some(actual) = &actual
            && expected.is_none_or(|expected| expected == actual)
            && self
                .unit_status
                .as_ref()
                .is_some_and(|status| status.active_state == "active")
            && link.is_some_and(|link| link.header.flags.contains(LinkFlags::Up))
            && self.link_settings.as_ref() == Some(&LinkSettings::for_config(actual))
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
        && link.attributes.iter().any(|attribute| {
            matches!(attribute, LinkAttribute::LinkInfo(info)
            if info.contains(&LinkInfo::Kind(InfoKind::Dummy)))
        })
}

pub(super) fn inspect(expected: Option<&ResolverConfig>) -> ResolverFileState {
    match run_native("inspection", DNS_INSPECTION_TIMEOUT, async {
        let (_, observation) = observe().await?;
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

fn read_unit(path: &Utf8Path) -> io::Result<Option<String>> {
    let descriptor = match open(
        path.as_std_path(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut file = UnitFile::from(descriptor);
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "resolver unit is not a regular file",
        ));
    }
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(Some(content))
}

async fn read_links() -> Result<Vec<LinkMessage>, PlatformError> {
    with_netlink(|handle| async move { query_links(&handle).await }).await
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

async fn observe() -> Result<(Connection, Observation), PlatformError> {
    let unit_content =
        read_unit(Utf8Path::new(SYSTEM_RESOLVER_TEST_PATH)).map_err(PlatformError::ResolverIo)?;
    let links = read_links().await?;
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
    let unit_status = read_unit_status(&systemd).await?;
    let resolved = resolved_manager(&connection).await?;
    let routing_domains = resolved.get_property("Domains").await.map_err(bus_error)?;
    let link_settings = if let Some(link) = pv_link(&links) {
        let index = i32::try_from(link.header.index)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
        read_link_settings(&resolved, index).await?
    } else {
        None
    };
    let observation = Observation {
        unit_content,
        unit_status,
        links,
        routing_domains,
        link_settings,
    };
    Ok((connection, observation))
}

async fn resolved_manager(connection: &Connection) -> Result<Proxy<'static>, PlatformError> {
    resolved_proxy(
        connection,
        "/org/freedesktop/resolve1"
            .try_into()
            .map_err(zbus::Error::from)
            .map_err(bus_error)?,
        "org.freedesktop.resolve1.Manager",
    )
    .await
}

async fn read_unit_status(manager: &Proxy<'_>) -> Result<Option<UnitStatus>, PlatformError> {
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
        fragment_path: fragment_path.into(),
        drop_in_paths: unit.get_property("DropInPaths").await.map_err(bus_error)?,
    }))
}

async fn resolved_proxy(
    connection: &Connection,
    path: OwnedObjectPath,
    interface: &'static str,
) -> Result<Proxy<'static>, PlatformError> {
    proxy::Builder::new(connection)
        .destination("org.freedesktop.resolve1")
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
) -> Result<Option<LinkSettings>, PlatformError> {
    let Some(path) = resolved_link_path(manager, index).await? else {
        return Ok(None);
    };
    let link = resolved_proxy(manager.connection(), path, "org.freedesktop.resolve1.Link").await?;
    Ok(Some(LinkSettings {
        dns: link.get_property("DNSEx").await.map_err(bus_error)?,
        domains: link.get_property("Domains").await.map_err(bus_error)?,
        default_route: link.get_property("DefaultRoute").await.map_err(bus_error)?,
        dnssec: link.get_property("DNSSEC").await.map_err(bus_error)?,
        dns_over_tls: link.get_property("DNSOverTLS").await.map_err(bus_error)?,
        llmnr: link.get_property("LLMNR").await.map_err(bus_error)?,
        multicast_dns: link.get_property("MulticastDNS").await.map_err(bus_error)?,
    }))
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
        let (connection, before) = observe().await?;
        before.require_owned()?;
        let resolved = resolved_manager(&connection).await?;
        with_netlink(|handle| async move {
            let links = query_links(&handle).await?;
            if owned_pv_link(&links)?.is_none() {
                handle
                    .link()
                    .add(
                        LinkDummy::new(LINK_NAME)
                            .append_extra_attribute(LinkAttribute::IfAlias(LINK_ALIAS.to_owned()))
                            .build(),
                    )
                    .execute()
                    .await
                    .map_err(netlink_error)?;
            }
            let links = query_links(&handle).await?;
            let link = owned_pv_link(&links)?.ok_or_else(|| {
                PlatformError::SystemIntegration("PV dummy link is missing after creation".to_owned())
            })?;
            let index = link.header.index;
            let resolved_index = i32::try_from(index)
                .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
            handle
                .link()
                .set(LinkUnspec::new_with_index(index).up().build())
                .execute()
                .await
                .map_err(netlink_error)?;
            // resolved processes the kernel link notification independently of the netlink ACK.
            while resolved_link_path(&resolved, resolved_index).await?.is_none() {
                Timer::after(Duration::from_millis(25)).await;
            }
            let settings = LinkSettings::for_config(&ResolverConfig::new(port));
            resolved
                .call::<_, _, ()>("SetLinkDefaultRoute", &(resolved_index, settings.default_route))
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
            let links = query_links(&handle).await?;
            let current_link = owned_pv_link(&links)?.filter(|link| {
                link.header.index == index && link.header.flags.contains(LinkFlags::Up)
            });
            if current_link.is_none()
                || read_link_settings(&resolved, resolved_index).await?.as_ref() != Some(&settings)
            {
                return Err(PlatformError::SystemIntegration(
                    "PV dummy link settings did not match after apply; inspect DNS state before retrying"
                        .to_owned(),
                ));
            }
            Ok(())
        })
        .await
    })
}

/// Reverts resolved settings and deletes only the fixed PV-owned dummy link. Requires root.
pub fn stop_resolver_link() -> Result<(), PlatformError> {
    require_root()?;
    run_native("link down", Duration::from_secs(10), stop_link())
}

async fn stop_link() -> Result<(), PlatformError> {
    let (connection, before) = observe().await?;
    before.require_owned()?;
    let resolved = resolved_manager(&connection).await?;
    with_netlink(|handle| async move {
        let links = query_links(&handle).await?;
        let Some(link) = owned_pv_link(&links)? else {
            return Ok(());
        };
        let index = link.header.index;
        let resolved_index = i32::try_from(index)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
        match resolved
            .call::<_, _, ()>("RevertLink", &(resolved_index,))
            .await
        {
            Ok(()) => {}
            Err(error) if method_error(&error, "org.freedesktop.resolve1.NoSuchLink") => {}
            Err(error) => return Err(bus_error(error)),
        }
        let links = query_links(&handle).await?;
        if let Some(link) = owned_pv_link(&links)? {
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
        if pv_link(&query_links(&handle).await?).is_some() {
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
        let (connection, before) = observe().await?;
        before.require_owned()?;
        let manager = system_manager_proxy(&connection).await.map_err(bus_error)?;
        write_unit(path, config)?;
        manager
            .call::<_, _, ()>("Reload", &())
            .await
            .map_err(bus_error)?;
        // Reload may select a newly visible fragment or drop-in. Check the manager's
        // actual selection before enablement or execution.
        let (_, reloaded) = observe().await?;
        reloaded.require_owned()?;
        manager
            .call::<_, _, (bool, Vec<(String, String, String)>)>(
                "EnableUnitFiles",
                &([UNIT_NAME].as_slice(), false, false),
            )
            .await
            .map_err(bus_error)?;
        dns_unit_job(&manager, "RestartUnit").await?;
        let (_, after) = observe().await?;
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
        let (connection, before) = observe().await?;
        before.require_owned()?;
        let manager = system_manager_proxy(&connection).await.map_err(bus_error)?;
        dns_unit_job(&manager, "StopUnit").await?;
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
        let (_, after) = observe().await?;
        match after.state(None) {
            ResolverFileState::Missing { .. } => Ok(()),
            state => Err(PlatformError::SystemIntegration(format!(
                "DNS state was not absent after removal: {state:?}"
            ))),
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

fn read_owned_unit(path: &Utf8Path) -> Result<Option<Vec<u8>>, PlatformError> {
    let content = root_files::read_root_file(path, 0o644)?;
    if let Some(content) = &content
        && !std::str::from_utf8(content)
            .ok()
            .is_some_and(|content| content.lines().any(|line| line.trim() == PV_MARKER))
    {
        return Err(conflict());
    }
    Ok(content)
}

fn require_no_candidate(path: &Utf8Path) -> Result<(), PlatformError> {
    let candidate = Utf8PathBuf::from(format!("{path}.candidate"));
    if root_files::read_root_file(&candidate, 0o644)?.is_some() {
        return Err(PlatformError::SystemIntegration(format!(
            "retained DNS unit candidate requires administrator inspection before retrying: {candidate}"
        )));
    }
    Ok(())
}

fn write_unit(path: &Utf8Path, config: &ResolverConfig) -> Result<(), PlatformError> {
    let parent = path.parent().ok_or_else(|| {
        PlatformError::SystemIntegration("DNS unit has no parent directory".to_owned())
    })?;
    root_files::validate_directory_chain(parent, false)?;
    read_owned_unit(path)?;
    require_no_candidate(path)?;
    let candidate = Utf8PathBuf::from(format!("{path}.candidate"));
    let content = render_unit(config);
    root_files::write_root_file(&candidate, content.as_bytes(), 0o644)?;
    root_files::rename(&candidate, path)?;
    if read_owned_unit(path)?.as_deref() != Some(content.as_bytes()) {
        return Err(PlatformError::SystemIntegration(
            "DNS unit bytes did not match after write".to_owned(),
        ));
    }
    Ok(())
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
        "Linux DNS routing conflicts with administrator configuration; inspect {SYSTEM_RESOLVER_TEST_PATH}, its systemd overrides, pv0 ownership, and other links' ~test routing domains; remove or rename the conflicting configuration manually before retrying",
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
    use anyhow::Result;
    use camino_tempfile::tempdir;
    use insta::assert_snapshot;
    use rtnetlink::LinkDummy;
    use rustix::fs::{CWD, mkfifoat};

    use super::*;

    fn current() -> Observation {
        let config = ResolverConfig::new(35353);
        Observation {
            unit_content: Some(render_unit(&config)),
            unit_status: Some(UnitStatus {
                active_state: "active".to_owned(),
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
            routing_domains: vec![(7, "test".to_owned(), true)],
            link_settings: Some(LinkSettings {
                dns: vec![(2, vec![127, 0, 0, 1], 35353, String::new())],
                domains: vec![("test".to_owned(), true)],
                default_route: false,
                dnssec: "no".to_owned(),
                dns_over_tls: "no".to_owned(),
                llmnr: "no".to_owned(),
                multicast_dns: "no".to_owned(),
            }),
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
        let mut missing_link = observation.clone();
        missing_link.links.clear();
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
        for index in [0, 9] {
            let mut other_route = Observation::default();
            other_route
                .routing_domains
                .push((index, "TeSt.".to_owned(), true));
            conflicts.push((
                "foreign routing domain without PV installation",
                other_route,
            ));
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
            assert!(observation.require_owned().is_err(), "{case}");
            assert_eq!(
                observation.state(None),
                ResolverFileState::Conflict {
                    path: SYSTEM_RESOLVER_TEST_PATH.into()
                },
                "{case}"
            );
        }
        let search_only = Observation {
            routing_domains: vec![(9, "test".to_owned(), false)],
            ..Observation::default()
        };
        assert_eq!(
            search_only.state(None),
            ResolverFileState::Missing {
                path: SYSTEM_RESOLVER_TEST_PATH.into()
            }
        );
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
        let links = run_native("inspection", Duration::from_secs(10), read_links())?;
        assert!(links.iter().any(|link| link.header.index > 0
            && link.header.flags.contains(LinkFlags::Loopback)
            && link.attributes.iter().any(
                |attribute| matches!(attribute, LinkAttribute::IfName(name) if name == "lo")
            )));
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
