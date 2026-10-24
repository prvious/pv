use std::collections::BTreeSet;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use zbus::zvariant::OwnedObjectPath;
use zbus::{Proxy, connection};

use super::{
    ActivePfRedirectInspection, PV_MARKER, PfFileState, PfRedirectConfig, classify_pv_file_state,
};
use crate::PlatformError;
use crate::systemd::{native_proxy, read_unit, within};

pub(super) mod root;
pub(crate) use root::MUTATION_TIMEOUT;

const PROXY_EXECUTABLE: &str = "/usr/lib/systemd/systemd-socket-proxyd";
pub(in crate::pf) const SYSTEM_DIRECTORY: &str = "/etc/systemd/system";
pub(crate) const INSPECTION_TIMEOUT: Duration = Duration::from_secs(5);

// org.freedesktop.systemd1.Service.ExecStart: a(sasbttttuii).
type ExecCommand = (String, Vec<String>, bool, u64, u64, u64, u64, u32, i32, i32);

struct Frontend {
    name: &'static str,
    label: &'static str,
    public_port: u16,
}

const HTTP: Frontend = Frontend {
    name: "pv-http",
    label: "HTTP",
    public_port: 80,
};
const HTTPS: Frontend = Frontend {
    name: "pv-https",
    label: "HTTPS",
    public_port: 443,
};

impl Frontend {
    fn socket_name(&self) -> String {
        format!("{}.socket", self.name)
    }

    fn service_name(&self) -> String {
        format!("{}.service", self.name)
    }

    fn render_socket(&self) -> String {
        format!(
            "{PV_MARKER}\n[Unit]\nDescription=PV {} listener\n\n[Socket]\nListenStream=127.0.0.1:{}\n\n[Install]\nWantedBy=sockets.target\n",
            self.label, self.public_port
        )
    }

    fn render_service(&self, port: u16) -> String {
        let socket = self.socket_name();
        format!(
            "{PV_MARKER}\n[Unit]\nDescription=PV {} forwarder\nRequires={socket}\nAfter={socket}\n\n[Service]\nExecStart={PROXY_EXECUTABLE} 127.0.0.1:{port}\nDynamicUser=yes\nPrivateTmp=yes\nProtectSystem=strict\nProtectHome=yes\nNoNewPrivileges=yes\n",
            self.label
        )
    }
}

pub(in crate::pf) fn render_units(config: &PfRedirectConfig) -> [(String, String); 4] {
    [
        (HTTP.socket_name(), HTTP.render_socket()),
        (HTTP.service_name(), HTTP.render_service(config.http_port)),
        (HTTPS.socket_name(), HTTPS.render_socket()),
        (
            HTTPS.service_name(),
            HTTPS.render_service(config.https_port),
        ),
    ]
}

pub(in crate::pf) fn unit_names() -> [String; 4] {
    [
        HTTP.socket_name(),
        HTTP.service_name(),
        HTTPS.socket_name(),
        HTTPS.service_name(),
    ]
}

pub(in crate::pf) fn inspect_unit(path: &Utf8Path, expected: Option<&str>) -> PfFileState<String> {
    let content = match read_unit(path) {
        Ok(Some(content)) => content,
        Ok(None) => {
            return PfFileState::Missing {
                path: path.to_owned(),
            };
        }
        Err(error) => {
            return PfFileState::Unreadable {
                path: path.to_owned(),
                message: error.to_string(),
            };
        }
    };
    if !content.lines().any(|line| line.trim() == PV_MARKER) {
        return PfFileState::Conflict {
            path: path.to_owned(),
        };
    }
    classify_pv_file_state(path, expected.map(str::to_owned).as_ref(), Some(content))
}

#[derive(Clone, Default)]
struct Unit {
    content: Option<String>,
    status: Option<UnitStatus>,
}

#[derive(Clone)]
struct UnitStatus {
    load_state: String,
    needs_reload: bool,
    active_state: String,
    fragment_path: Utf8PathBuf,
    drop_in_paths: Vec<String>,
}

impl Unit {
    fn owned(&self, name: &str) -> bool {
        self.content
            .as_ref()
            .is_some_and(|content| content.lines().any(|line| line.trim() == PV_MARKER))
            && self.status.as_ref().is_none_or(|status| {
                status.fragment_path == Utf8Path::new(SYSTEM_DIRECTORY).join(name)
                    && status.drop_in_paths.is_empty()
            })
    }

    fn active(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| status.active_state == "active")
    }

    fn inactive(&self) -> bool {
        self.status
            .as_ref()
            .is_none_or(|status| status.active_state == "inactive")
    }
}

#[derive(Clone, Default)]
struct Forwarder {
    socket: Unit,
    service: Unit,
    listen: Vec<(String, String)>,
    commands: Vec<ExecCommand>,
    execution_matches: bool,
}

impl Forwarder {
    fn owned(&self, frontend: &Frontend) -> bool {
        self.socket.owned(&frontend.socket_name()) && self.service.owned(&frontend.service_name())
    }

    fn target(&self) -> Option<u16> {
        if !self.service.inactive() && !(self.service.active() && self.execution_matches) {
            return None;
        }
        self.configured_target()
    }

    fn configured_target(&self) -> Option<u16> {
        let [(executable, arguments, ignore_failure, ..)] = self.commands.as_slice() else {
            return None;
        };
        let [argument_zero, address] = arguments.as_slice() else {
            return None;
        };
        let port = address.strip_prefix("127.0.0.1:")?.parse::<u16>().ok()?;
        (executable == PROXY_EXECUTABLE
            && argument_zero == PROXY_EXECUTABLE
            && !ignore_failure
            && port >= 1024
            && address == &format!("127.0.0.1:{port}"))
            .then_some(port)
    }

    fn listener_matches(&self, frontend: &Frontend) -> bool {
        self.socket.active()
            && self
                .socket
                .status
                .as_ref()
                .is_some_and(|status| status.load_state == "loaded" && !status.needs_reload)
            && self.listen
                == [(
                    "Stream".to_owned(),
                    format!("127.0.0.1:{}", frontend.public_port),
                )]
            && self.service.status.as_ref().is_some_and(|status| {
                status.load_state == "loaded"
                    && !status.needs_reload
                    && matches!(status.active_state.as_str(), "active" | "inactive")
            })
    }

    fn files_current(&self, frontend: &Frontend, port: u16) -> bool {
        self.owned(frontend)
            && self.socket.content.as_ref() == Some(&frontend.render_socket())
            && self.service.content.as_ref() == Some(&frontend.render_service(port))
    }
}

fn inspection(http: &Forwarder, https: &Forwarder) -> ActivePfRedirectInspection {
    let pv_config = http
        .target()
        .zip(https.target())
        .and_then(|(http_port, https_port)| {
            (http.listener_matches(&HTTP)
                && https.listener_matches(&HTTPS)
                && http_port != https_port)
                .then_some(PfRedirectConfig::new(http_port, https_port))
        });
    let pf_enabled = pv_config.as_ref().is_some_and(|config| {
        http.files_current(&HTTP, config.http_port)
            && https.files_current(&HTTPS, config.https_port)
    });
    let mut resolved_target_ports = BTreeSet::new();
    let mut has_unresolved_redirect_targets = false;
    for forwarder in [http, https] {
        if let Some(port) = forwarder.target() {
            resolved_target_ports.insert(port);
        } else if !forwarder.commands.is_empty() || !forwarder.service.inactive() {
            has_unresolved_redirect_targets = true;
        }
    }
    let pv_anchor_has_unparsed_rules =
        pv_config.is_none() && (!http.socket.inactive() || !https.socket.inactive());
    ActivePfRedirectInspection {
        pf_enabled,
        pv_config,
        pv_anchor_has_unparsed_rules,
        resolved_target_ports,
        has_unresolved_redirect_targets,
    }
}

pub(super) fn inspect() -> Result<ActivePfRedirectInspection, PlatformError> {
    futures_lite::future::block_on(within(INSPECTION_TIMEOUT, async {
        let manager = inspection_manager().await?;
        let http = observe(&manager, &HTTP).await?;
        let https = observe(&manager, &HTTPS).await?;
        complete_inspection(http, https, Utf8Path::new(SYSTEM_DIRECTORY))
    }))
    .ok_or_else(|| {
        PlatformError::SystemIntegration("systemd socket proxy inspection timed out".to_owned())
    })?
}

pub(in crate::pf) fn units_absent() -> Result<bool, PlatformError> {
    futures_lite::future::block_on(within(INSPECTION_TIMEOUT, async {
        let manager = inspection_manager().await?;
        for name in unit_names() {
            if query_unit(&manager, &name, "GetUnit")
                .await?
                .0
                .status
                .is_some()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }))
    .ok_or_else(|| {
        PlatformError::SystemIntegration(
            "systemd socket proxy absence inspection timed out".to_owned(),
        )
    })?
}

pub(in crate::pf) fn repairable_ports() -> Result<BTreeSet<u16>, PlatformError> {
    futures_lite::future::block_on(within(INSPECTION_TIMEOUT, async {
        let manager = inspection_manager().await?;
        let mut ports = BTreeSet::new();
        for frontend in [&HTTP, &HTTPS] {
            let mut forwarder = Forwarder {
                socket: query_unit(&manager, &frontend.socket_name(), "GetUnit")
                    .await?
                    .0,
                service: query_unit(&manager, &frontend.service_name(), "GetUnit")
                    .await?
                    .0,
                ..Forwarder::default()
            };
            read_files(&mut forwarder, frontend, Utf8Path::new(SYSTEM_DIRECTORY))?;
            if forwarder.owned(frontend) && forwarder.socket.active() {
                ports.insert(frontend.public_port);
            }
        }
        Ok(ports)
    }))
    .ok_or_else(|| {
        PlatformError::SystemIntegration(
            "systemd socket proxy ownership inspection timed out".to_owned(),
        )
    })?
}

async fn inspection_manager() -> Result<Proxy<'static>, PlatformError> {
    let connection = connection::Builder::address("unix:path=/run/dbus/system_bus_socket")
        .map_err(bus_error)?
        .build()
        .await
        .map_err(bus_error)?;
    native_proxy(
        &connection,
        "/org/freedesktop/systemd1"
            .try_into()
            .map_err(zbus::Error::from)
            .map_err(bus_error)?,
        "org.freedesktop.systemd1.Manager",
    )
    .await
    .map_err(bus_error)
}

fn read_files(
    forwarder: &mut Forwarder,
    frontend: &Frontend,
    directory: &Utf8Path,
) -> Result<(), PlatformError> {
    forwarder.socket.content =
        read_unit(&directory.join(frontend.socket_name())).map_err(PlatformError::PortProxyIo)?;
    forwarder.service.content =
        read_unit(&directory.join(frontend.service_name())).map_err(PlatformError::PortProxyIo)?;
    Ok(())
}

fn complete_inspection(
    mut http: Forwarder,
    mut https: Forwarder,
    directory: &Utf8Path,
) -> Result<ActivePfRedirectInspection, PlatformError> {
    let loaded = inspection(&http, &https);
    if loaded.pv_config.is_none() {
        return Ok(loaded);
    }
    for (frontend, forwarder) in [(&HTTP, &mut http), (&HTTPS, &mut https)] {
        read_files(forwarder, frontend, directory)?;
    }
    Ok(inspection(&http, &https))
}

async fn observe(manager: &Proxy<'_>, frontend: &Frontend) -> Result<Forwarder, PlatformError> {
    let (socket, socket_path) = query_unit(manager, &frontend.socket_name(), "GetUnit").await?;
    let (service, service_path) = query_unit(manager, &frontend.service_name(), "GetUnit").await?;
    let listen = if let Some(path) = socket_path {
        native_proxy(
            manager.connection(),
            path,
            "org.freedesktop.systemd1.Socket",
        )
        .await
        .map_err(bus_error)?
        .get_property("Listen")
        .await
        .map_err(bus_error)?
    } else {
        Vec::new()
    };
    let mut main_pid = 0;
    let commands = if let Some(path) = service_path {
        let service = native_proxy(
            manager.connection(),
            path,
            "org.freedesktop.systemd1.Service",
        )
        .await
        .map_err(bus_error)?;
        main_pid = service.get_property("MainPID").await.map_err(bus_error)?;
        service.get_property("ExecStart").await.map_err(bus_error)?
    } else {
        Vec::new()
    };
    let mut forwarder = Forwarder {
        socket,
        service,
        listen,
        commands,
        execution_matches: false,
    };
    if forwarder.service.active()
        && forwarder.configured_target().is_some()
        && let [command] = forwarder.commands.as_slice()
    {
        forwarder.execution_matches = executing_command_matches(command, main_pid)?;
    }
    Ok(forwarder)
}

fn executing_command_matches(command: &ExecCommand, main_pid: u32) -> Result<bool, PlatformError> {
    if main_pid == 0 {
        return Ok(false);
    }
    if command.7 == main_pid {
        return Ok(true);
    }
    // A manager reload can erase ExecStart's execution metadata. Compare the
    // running argv before accepting its newly loaded target as current.
    let Some((argument_zero, arguments)) = crate::process::inspect_process_command_line(main_pid)?
    else {
        return Ok(false);
    };
    Ok(command
        .1
        .iter()
        .map(String::as_str)
        .eq(std::iter::once(argument_zero.as_str()).chain(arguments.iter().map(String::as_str))))
}

async fn query_unit(
    manager: &Proxy<'_>,
    name: &str,
    method: &str,
) -> Result<(Unit, Option<OwnedObjectPath>), PlatformError> {
    let path: OwnedObjectPath = match manager.call(method, &(name,)).await {
        Ok(path) => path,
        Err(zbus::Error::MethodError(error, _, _))
            if error.as_str() == "org.freedesktop.systemd1.NoSuchUnit" =>
        {
            return Ok((Unit::default(), None));
        }
        Err(error) => return Err(bus_error(error)),
    };
    let unit = native_proxy(
        manager.connection(),
        path.clone(),
        "org.freedesktop.systemd1.Unit",
    )
    .await
    .map_err(bus_error)?;
    let status = UnitStatus {
        load_state: unit.get_property("LoadState").await.map_err(bus_error)?,
        needs_reload: unit
            .get_property("NeedDaemonReload")
            .await
            .map_err(bus_error)?,
        active_state: unit.get_property("ActiveState").await.map_err(bus_error)?,
        fragment_path: unit
            .get_property::<String>("FragmentPath")
            .await
            .map_err(bus_error)?
            .into(),
        drop_in_paths: unit.get_property("DropInPaths").await.map_err(bus_error)?,
    };
    if status.load_state == "not-found"
        && status.active_state == "inactive"
        && status.fragment_path.as_str().is_empty()
    {
        Ok((Unit::default(), None))
    } else {
        Ok((
            Unit {
                status: Some(status),
                ..Unit::default()
            },
            Some(path),
        ))
    }
}

fn bus_error(source: zbus::Error) -> PlatformError {
    PlatformError::PortProxyBus(Box::new(source))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::process;

    use anyhow::{Context, Result, anyhow};
    use camino_tempfile::tempdir;
    use insta::assert_debug_snapshot;
    use state::fs;
    use zbus::zvariant::Type;

    use super::{
        ExecCommand, Forwarder, Frontend, HTTP, HTTPS, PROXY_EXECUTABLE, SYSTEM_DIRECTORY, Unit,
        UnitStatus, complete_inspection, executing_command_matches, inspection,
    };
    use crate::{PfRedirectConfig, PlatformError, inspect_process_identity};

    fn current(frontend: &Frontend, port: u16) -> Forwarder {
        let unit = |name: &str, content| Unit {
            content: Some(content),
            status: Some(UnitStatus {
                load_state: "loaded".to_owned(),
                needs_reload: false,
                active_state: "active".to_owned(),
                fragment_path: format!("{SYSTEM_DIRECTORY}/{name}").into(),
                drop_in_paths: Vec::new(),
            }),
        };
        Forwarder {
            socket: unit(&frontend.socket_name(), frontend.render_socket()),
            service: unit(&frontend.service_name(), frontend.render_service(port)),
            listen: vec![(
                "Stream".to_owned(),
                format!("127.0.0.1:{}", frontend.public_port),
            )],
            commands: vec![command(port)],
            execution_matches: true,
        }
    }

    fn command(port: u16) -> ExecCommand {
        (
            PROXY_EXECUTABLE.to_owned(),
            vec![PROXY_EXECUTABLE.to_owned(), format!("127.0.0.1:{port}")],
            false,
            1,
            2,
            3,
            4,
            17,
            0,
            0,
        )
    }

    #[test]
    fn fixed_units_forward_only_loopback_with_dynamic_user_hardening() {
        assert_debug_snapshot!((
            HTTP.socket_name(),
            HTTP.render_socket(),
            HTTP.service_name(),
            HTTP.render_service(48080),
            HTTPS.socket_name(),
            HTTPS.render_socket(),
            HTTPS.service_name(),
            HTTPS.render_service(48443),
        ));
    }

    #[test]
    fn owned_units_allow_repair_without_proving_current_routing() -> Result<()> {
        let mut owned = current(&HTTP, 45000);
        owned.listen.clear();
        owned.commands.clear();
        assert!(owned.owned(&HTTP));
        assert!(
            inspection(&owned, &Forwarder::default())
                .pv_config
                .is_none()
        );

        let mut foreign = owned.clone();
        foreign.socket.content = Some("administrator unit".to_owned());
        let mut foreign_fragment = owned.clone();
        foreign_fragment
            .socket
            .status
            .as_mut()
            .context("fixture socket missing")?
            .fragment_path = "/run/systemd/system/pv-http.socket".into();
        let mut overridden = owned.clone();
        overridden
            .service
            .status
            .as_mut()
            .context("fixture service missing")?
            .drop_in_paths
            .push("/etc/systemd/system/pv-http.service.d/administrator.conf".to_owned());
        for rejected in [
            Forwarder {
                socket: Unit::default(),
                ..owned.clone()
            },
            Forwarder {
                service: Unit::default(),
                ..owned
            },
            foreign,
            foreign_fragment,
            overridden,
        ] {
            assert!(!rejected.owned(&HTTP));
        }
        Ok(())
    }

    #[test]
    fn native_evidence_distinguishes_exact_absent_partial_and_retained_targets() {
        let http = current(&HTTP, 48080);
        let https = current(&HTTPS, 48443);
        let exact = inspection(&http, &https);
        assert!(exact.pf_enabled);
        assert_eq!(exact.pv_config, Some(PfRedirectConfig::new(48080, 48443)));
        assert_eq!(exact.resolved_target_ports, BTreeSet::from([48080, 48443]));
        assert!(!exact.pv_anchor_has_unparsed_rules);
        assert!(!exact.has_unresolved_redirect_targets);

        let missing = inspection(&Forwarder::default(), &Forwarder::default());
        assert!(!missing.pf_enabled);
        assert!(missing.pv_config.is_none());
        assert!(missing.resolved_target_ports.is_empty());
        assert!(!missing.pv_anchor_has_unparsed_rules);
        assert!(!missing.has_unresolved_redirect_targets);

        let mut unverified = http.clone();
        unverified.execution_matches = false;
        if let Some(status) = &mut unverified.socket.status {
            status.active_state = "inactive".to_owned();
        }
        let uncertain = inspection(&unverified, &Forwarder::default());
        assert!(uncertain.pv_config.is_none());
        assert!(uncertain.has_unresolved_redirect_targets);

        let partial = inspection(&http, &Forwarder::default());
        assert!(partial.pv_config.is_none());
        assert!(partial.pv_anchor_has_unparsed_rules);
        assert_eq!(partial.resolved_target_ports, BTreeSet::from([48080]));

        let mut stopped_http = http.clone();
        let mut stopped_https = https.clone();
        for forwarder in [&mut stopped_http, &mut stopped_https] {
            for unit in [&mut forwarder.socket, &mut forwarder.service] {
                if let Some(status) = &mut unit.status {
                    status.active_state = "inactive".to_owned();
                }
            }
        }
        let stopped = inspection(&stopped_http, &stopped_https);
        assert!(stopped.pv_config.is_none());
        assert!(!stopped.pv_anchor_has_unparsed_rules);
        assert_eq!(
            stopped.resolved_target_ports,
            BTreeSet::from([48080, 48443])
        );

        // An inactive forwarder is activated by its socket after reboot.
        if let Some(status) = &mut stopped_http.socket.status {
            status.active_state = "active".to_owned();
        }
        if let Some(status) = &mut stopped_https.socket.status {
            status.active_state = "active".to_owned();
        }
        assert!(inspection(&stopped_http, &stopped_https).pf_enabled);
    }

    #[test]
    fn mismatched_loaded_state_and_noncurrent_units_never_prove_active() {
        let http = current(&HTTP, 48080);
        let https = current(&HTTPS, 48443);
        let mut variants = Vec::new();
        let mut wrong_listen = http.clone();
        wrong_listen.listen[0].1 = "0.0.0.0:80".to_owned();
        variants.push(("wildcard listener", wrong_listen));
        let mut datagram = http.clone();
        datagram.listen[0].0 = "Datagram".to_owned();
        variants.push(("wrong socket type", datagram));
        let mut extra_listen = http.clone();
        extra_listen
            .listen
            .push(("Stream".to_owned(), "127.0.0.1:8080".to_owned()));
        variants.push(("extra listener", extra_listen));
        let mut missing_socket = http.clone();
        missing_socket.socket.status = None;
        variants.push(("missing socket", missing_socket));
        let mut wrong_command = http.clone();
        wrong_command.commands[0].0 = "/usr/bin/other".to_owned();
        variants.push(("wrong executable", wrong_command));
        let mut duplicate_target = http.clone();
        duplicate_target.commands = vec![command(48443)];
        variants.push(("duplicate backend", duplicate_target));
        for socket in [true, false] {
            for needs_reload in [true, false] {
                let mut changed = http.clone();
                let unit = if socket {
                    &mut changed.socket
                } else {
                    &mut changed.service
                };
                if let Some(status) = &mut unit.status {
                    status.needs_reload = needs_reload;
                    if !needs_reload {
                        status.load_state = "error".to_owned();
                    }
                }
                variants.push((
                    if needs_reload {
                        "stale loaded definition"
                    } else {
                        "load failure"
                    },
                    changed,
                ));
            }
        }
        for (name, forwarder) in variants {
            let actual = inspection(&forwarder, &https);
            assert!(!actual.pf_enabled, "{name}");
            assert!(actual.pv_config.is_none(), "{name}");
            assert!(actual.pv_anchor_has_unparsed_rules, "{name}");
        }

        for name in [
            "missing file",
            "stale file",
            "foreign fragment",
            "drop-in",
            "failed service",
        ] {
            let mut changed = http.clone();
            match name {
                "missing file" => changed.socket.content = None,
                "stale file" => changed.service.content = Some("# Managed by PV\n".to_owned()),
                "foreign fragment" => {
                    if let Some(status) = &mut changed.service.status {
                        status.fragment_path = "/run/systemd/system/pv-http.service".into();
                    }
                }
                "drop-in" => {
                    if let Some(status) = &mut changed.socket.status {
                        status
                            .drop_in_paths
                            .push("/etc/systemd/system/pv-http.socket.d/foreign.conf".to_owned());
                    }
                }
                "failed service" => {
                    if let Some(status) = &mut changed.service.status {
                        status.active_state = "failed".to_owned();
                    }
                }
                _ => {}
            }
            assert!(!inspection(&changed, &https).pf_enabled, "{name}");
        }
    }

    #[test]
    fn unsafe_unit_files_cannot_hide_native_absence_or_prove_active() -> Result<()> {
        let directory = tempdir()?;
        let foreign = directory.path().join("foreign");
        fs::write_sensitive_file(&foreign, "foreign unit")?;
        fs::symlink_file(&foreign, &directory.path().join(HTTP.socket_name()))?;
        let absent =
            complete_inspection(Forwarder::default(), Forwarder::default(), directory.path())?;
        assert!(absent.pv_config.is_none());
        assert!(absent.resolved_target_ports.is_empty());
        assert!(!absent.pv_anchor_has_unparsed_rules);
        assert!(!absent.has_unresolved_redirect_targets);
        let retained = complete_inspection(
            current(&HTTP, 48080),
            Forwarder::default(),
            directory.path(),
        )?;
        assert_eq!(retained.resolved_target_ports, BTreeSet::from([48080]));
        assert!(matches!(
            complete_inspection(
                current(&HTTP, 48080),
                current(&HTTPS, 48443),
                directory.path()
            ),
            Err(PlatformError::PortProxyIo(_))
        ));
        Ok(())
    }

    #[test]
    fn active_invocation_uses_native_arguments_after_execution_metadata_is_lost() -> Result<()> {
        let pid = process::id();
        let identity =
            inspect_process_identity(pid)?.ok_or_else(|| anyhow!("test process disappeared"))?;
        let mut command = command(48080);
        command.1 = std::iter::once(identity.argument_zero)
            .chain(identity.arguments)
            .collect();
        command.7 = 0;
        assert!(executing_command_matches(&command, pid)?);
        command.1.push(String::new());
        assert!(!executing_command_matches(&command, pid)?);
        command.1.pop();
        command.1[0] = "other executable".to_owned();
        assert!(!executing_command_matches(&command, pid)?);
        assert!(!executing_command_matches(&command, 0)?);
        command.7 = pid;
        assert!(executing_command_matches(&command, pid)?);
        Ok(())
    }

    #[test]
    fn native_execstart_shape_and_unresolved_commands_preserve_uncertainty() {
        assert_eq!(
            <Vec<ExecCommand> as Type>::SIGNATURE.to_string(),
            "a(sasbttttuii)"
        );
        let mut forwarder = current(&HTTP, 48080);
        assert_eq!(forwarder.target(), Some(48080));
        let mut malformed = Vec::new();
        let mut missing_argv = command(48080);
        missing_argv.1.clear();
        malformed.push(vec![missing_argv]);
        let mut wrong_argv_zero = command(48080);
        wrong_argv_zero.1[0] = "other".to_owned();
        malformed.push(vec![wrong_argv_zero]);
        let mut nonloopback = command(48080);
        nonloopback.1[1] = "192.0.2.1:48080".to_owned();
        malformed.push(vec![nonloopback]);
        let mut bad_port = command(48080);
        bad_port.1[1] = "127.0.0.1:nope".to_owned();
        malformed.push(vec![bad_port]);
        let mut ignored_failure = command(48080);
        ignored_failure.2 = true;
        malformed.push(vec![ignored_failure]);
        malformed.push(vec![command(80)]);
        malformed.push(vec![command(48080), command(48443)]);
        for state in ["active", "inactive"] {
            if let Some(status) = &mut forwarder.service.status {
                status.active_state = state.to_owned();
            }
            for commands in &malformed {
                forwarder.commands = commands.clone();
                assert!(forwarder.target().is_none());
                assert!(
                    inspection(&forwarder, &Forwarder::default()).has_unresolved_redirect_targets,
                    "{state}"
                );
            }
        }
        if let Some(status) = &mut forwarder.service.status {
            status.active_state = "active".to_owned();
        }
        forwarder.commands.clear();
        assert!(inspection(&forwarder, &Forwarder::default()).has_unresolved_redirect_targets);
        if let Some(status) = &mut forwarder.service.status {
            status.active_state = "inactive".to_owned();
        }
        assert!(!inspection(&forwarder, &Forwarder::default()).has_unresolved_redirect_targets);
    }
}
