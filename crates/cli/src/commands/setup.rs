use std::io;
use std::io::Write;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use platform::{CaFileState, PfFileState, ResolverConfig, ResolverFileState, TrustDomainState};
use resources::{
    ArtifactManifest, ArtifactManifestCache, ArtifactManifestSource, ResourceHttpClient,
    ResourceName, TargetPlatform, TrackName, TrackSelector, UreqResourceHttpClient,
};
use state::{Database, ManagedResourceDesiredState, PvPaths, StateError};

use super::{ca, daemon as daemon_command, dns, ports};
use crate::args::{SetupArgs, UninstallArgs};
use crate::environment::{Environment, artifact_manifest_url};
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};
use crate::shell::Shell;

const PV_ENV_START: &str = "# >>> PV ENV";
const PV_ENV_END: &str = "# <<< PV ENV";
#[cfg(unix)]
const SHIM_FILE_MODE: u32 = 0o700;

const DEFAULT_SETUP_RESOURCES: &[SetupResourceDefault] = &[
    SetupResourceDefault::manifest_default("frankenphp"),
    SetupResourceDefault::manifest_default("php"),
    SetupResourceDefault::manifest_default("mysql"),
    SetupResourceDefault::manifest_default("postgres"),
    SetupResourceDefault::manifest_default("redis"),
    SetupResourceDefault::manifest_default("mailpit"),
    SetupResourceDefault::manifest_default("rustfs"),
    SetupResourceDefault::concrete("composer", "2"),
];

#[derive(Clone, Copy, Debug)]
struct SetupResourceDefault {
    resource_name: &'static str,
    track: SetupResourceTrackDefault,
}

#[derive(Clone, Copy, Debug)]
enum SetupResourceTrackDefault {
    ManifestDefault,
    Concrete(&'static str),
}

#[derive(Clone, Debug)]
struct SetupResourcePlan {
    resource_name: ResourceName,
    track: TrackName,
}

impl SetupResourceDefault {
    const fn manifest_default(resource_name: &'static str) -> Self {
        Self {
            resource_name,
            track: SetupResourceTrackDefault::ManifestDefault,
        }
    }

    const fn concrete(resource_name: &'static str, track: &'static str) -> Self {
        Self {
            resource_name,
            track: SetupResourceTrackDefault::Concrete(track),
        }
    }
}

pub(crate) fn setup(
    args: SetupArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    state::fs::ensure_layout(&paths)?;

    {
        let mut output = Output::new(stdout, OutputMode::plain());
        output.line("PV setup")?;
        output.line(&format!("Ensured PV state layout: {}", paths.root()))?;
    }
    install_command_shims(environment, &paths)?;
    let default_resource_plan = refresh_setup_artifact_manifest(environment, &paths, stdout)?;

    if args.non_interactive && setup_requires_privileged_auth(environment, &paths)? {
        let mut output = Output::new(stdout, OutputMode::plain());
        output.line(
            "pv setup --non-interactive requires macOS authentication for system integrations.",
        )?;
        output.line("Run `pv setup` interactively, then rerun with `--non-interactive`.")?;

        return Ok(ExitCode::FAILURE);
    }

    if configure_shell_integration(&args, environment, &paths, stdout)? != ExitCode::SUCCESS {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("DNS resolver setup", stdout, |stdout| {
        dns::install_config_only(environment, stdout)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("port redirect setup", stdout, |stdout| {
        ports::install(environment, stdout)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("CA trust setup", stdout, |stdout| {
        ca::trust_with_mode(
            environment,
            stdout,
            setup_privilege_mode(args.non_interactive),
        )
    })? {
        return Ok(ExitCode::FAILURE);
    }
    record_default_resource_desired_state(&paths, &default_resource_plan)?;
    if !run_required_step("daemon registration", stdout, |stdout| {
        daemon_command::enable_without_reconciliation(environment, stdout)
    })? {
        return Ok(ExitCode::FAILURE);
    }

    let completed = ::daemon::run_job_blocking(paths, "reconcile", "system")?;
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line(&format!(
        "System reconciliation completed: {}",
        completed.summary
    ))?;
    output.line("PV setup complete")?;

    Ok(ExitCode::SUCCESS)
}

fn refresh_setup_artifact_manifest(
    environment: &impl Environment,
    paths: &PvPaths,
    stdout: &mut impl Write,
) -> Result<Vec<SetupResourcePlan>, ExecuteError> {
    let cache = ArtifactManifestCache::new(paths.downloads());
    let manifest_url = artifact_manifest_url(environment);

    let refresh =
        with_resource_http_client(environment, |client| cache.refresh(&manifest_url, client))?;

    if let ArtifactManifestSource::Cached { reason } = refresh.source() {
        let mut output = Output::new(stdout, OutputMode::plain());
        output.line(&format!(
            "warning: artifact manifest refresh failed ({reason}); using cached manifest at {}",
            cache.path()
        ))?;
    }

    resolve_default_resource_plan(refresh.manifest(), target_platform(environment))
}

fn resolve_default_resource_plan(
    manifest: &ArtifactManifest,
    target_platform: TargetPlatform,
) -> Result<Vec<SetupResourcePlan>, ExecuteError> {
    let php_resource = ResourceName::new("php")?;
    let php_default_track = manifest.resolve_track(&php_resource, TrackSelector::Latest)?;
    let mut plan = Vec::new();

    for resource_default in DEFAULT_SETUP_RESOURCES {
        let resource_name = ResourceName::new(resource_default.resource_name)?;
        let track = if resource_name.as_str() == "frankenphp" {
            manifest.resolve_track(
                &resource_name,
                TrackSelector::Track(php_default_track.clone()),
            )?
        } else {
            let track_selector = match resource_default.track {
                SetupResourceTrackDefault::ManifestDefault => TrackSelector::Latest,
                SetupResourceTrackDefault::Concrete(track) => {
                    TrackSelector::Track(TrackName::new(track)?)
                }
            };
            manifest.resolve_track(&resource_name, track_selector)?
        };

        manifest.select_latest(&resource_name, track, target_platform)?;

        plan.push(SetupResourcePlan {
            resource_name,
            track: track.clone(),
        });
    }

    Ok(plan)
}

fn with_resource_http_client<T>(
    environment: &impl Environment,
    operation: impl FnOnce(&dyn ResourceHttpClient) -> resources::Result<T>,
) -> resources::Result<T> {
    if let Some(client) = environment.resource_http_client() {
        return operation(client);
    }

    let client = UreqResourceHttpClient::default();

    operation(&client)
}

fn target_platform(environment: &impl Environment) -> TargetPlatform {
    environment
        .target_platform()
        .unwrap_or_else(current_target_platform)
}

fn current_target_platform() -> TargetPlatform {
    if cfg!(target_arch = "aarch64") {
        TargetPlatform::DarwinArm64
    } else {
        TargetPlatform::DarwinAmd64
    }
}

fn install_command_shims(
    environment: &impl Environment,
    paths: &PvPaths,
) -> Result<(), ExecuteError> {
    let pv_executable = current_executable(environment)?;

    for shim in [
        CommandShim {
            name: "php",
            command: "shim:php",
        },
        CommandShim {
            name: "composer",
            command: "shim:composer",
        },
    ] {
        let path = paths.bin().join(shim.name);
        let content = format!(
            "#!/bin/sh\nexec {} {} \"$@\"\n",
            shell_quote(pv_executable.as_str()),
            shim.command
        );

        write_executable_file(&path, &content)?;
    }

    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct CommandShim {
    name: &'static str,
    command: &'static str,
}

pub(crate) fn uninstall(
    args: UninstallArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;

    if args.prune && !args.force && !confirm_prune(environment, stdout)? {
        return Ok(ExitCode::FAILURE);
    }

    {
        let mut output = Output::new(stdout, OutputMode::plain());
        output.line("PV uninstall")?;
    }

    if !run_required_step("daemon removal", stdout, |stdout| {
        daemon_command::disable(environment, stdout)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("DNS resolver removal", stdout, |stdout| {
        dns::uninstall(environment, stdout)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("port redirect removal", stdout, |stdout| {
        ports::uninstall(environment, stdout)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("CA trust removal", stdout, |stdout| {
        untrust_ca_for_uninstall(environment, stdout)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if remove_shell_integration(environment, &paths, stdout)? != ExitCode::SUCCESS {
        return Ok(ExitCode::FAILURE);
    }

    if args.prune {
        prune_state(&paths, stdout)?;
    } else {
        remove_default_state(&paths, stdout)?;
    }

    let mut output = Output::new(stdout, OutputMode::plain());
    output.line("PV uninstall complete")?;

    Ok(ExitCode::SUCCESS)
}

fn run_required_step<Writer>(
    label: &str,
    stdout: &mut Writer,
    command: impl FnOnce(&mut Writer) -> Result<ExitCode, ExecuteError>,
) -> Result<bool, ExecuteError>
where
    Writer: Write,
{
    let exit_code = command(stdout)?;
    if exit_code == ExitCode::SUCCESS {
        return Ok(true);
    }

    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!("PV stopped during {label}."))?;

    Ok(false)
}

fn setup_requires_privileged_auth(
    environment: &impl Environment,
    paths: &PvPaths,
) -> Result<bool, ExecuteError> {
    if dns_setup_requires_privileged_auth(environment, paths)? {
        return Ok(true);
    }
    if ports_setup_requires_privileged_auth(environment, paths)? {
        return Ok(true);
    }
    if ca_setup_requires_privileged_auth(environment, paths)? {
        return Ok(true);
    }

    Ok(false)
}

fn dns_setup_requires_privileged_auth(
    environment: &impl Environment,
    paths: &PvPaths,
) -> Result<bool, ExecuteError> {
    let prepared_state = platform::inspect_resolver_file(&paths.resolver_config(), None);
    let expected_config = resolver_config_from_state(&prepared_state);
    let system_path = resolver_test_path(environment)?;
    let system_state = platform::inspect_resolver_file(&system_path, expected_config.as_ref());

    Ok(matches!(
        system_state,
        ResolverFileState::Missing { .. } | ResolverFileState::Stale { .. }
    ))
}

fn ports_setup_requires_privileged_auth(
    environment: &impl Environment,
    paths: &PvPaths,
) -> Result<bool, ExecuteError> {
    let prepared_anchor_state = platform::inspect_pf_anchor_file(&paths.pf_anchor_config(), None);
    let prepared_reference_state =
        platform::inspect_pf_conf_reference(&paths.pf_conf_reference_config(), None);
    let expected_anchor = pf_config_from_anchor_state(&prepared_anchor_state);
    let expected_reference = pf_reference_from_state(&prepared_reference_state);
    let system_anchor_path = pf_anchor_path(environment)?;
    let system_pf_conf_path = pf_conf_path(environment)?;
    let system_anchor_state =
        platform::inspect_pf_anchor_file(&system_anchor_path, expected_anchor.as_ref());
    let system_reference_state =
        platform::inspect_pf_conf_reference(&system_pf_conf_path, expected_reference.as_ref());

    if matches!(
        system_anchor_state,
        PfFileState::Missing { .. } | PfFileState::Stale { .. }
    ) || matches!(
        system_reference_state,
        PfFileState::Missing { .. } | PfFileState::Stale { .. }
    ) {
        return Ok(true);
    }

    let Some(expected_anchor) = expected_anchor else {
        return Ok(false);
    };
    let active_config = environment.active_pf_redirect_config()?;

    Ok(active_config.as_ref() != Some(&expected_anchor))
}

fn ca_setup_requires_privileged_auth(
    environment: &impl Environment,
    paths: &PvPaths,
) -> Result<bool, ExecuteError> {
    let local_state =
        platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let Some(metadata) = metadata_from_local_ca_state(&local_state) else {
        return Ok(true);
    };
    let trust_state = system_trust_state(environment, Some(&metadata));

    Ok(!matches!(trust_state, TrustDomainState::Current { .. }))
}

fn resolver_config_from_state(state: &ResolverFileState) -> Option<ResolverConfig> {
    match state {
        ResolverFileState::Current { port, .. }
        | ResolverFileState::Stale {
            actual_port: Some(port),
            ..
        } => Some(ResolverConfig::new(*port)),
        ResolverFileState::Missing { .. }
        | ResolverFileState::Stale {
            actual_port: None, ..
        }
        | ResolverFileState::Conflict { .. }
        | ResolverFileState::Unreadable { .. } => None,
    }
}

fn pf_config_from_anchor_state(
    state: &PfFileState<platform::PfRedirectConfig>,
) -> Option<platform::PfRedirectConfig> {
    match state {
        PfFileState::Current { value, .. }
        | PfFileState::Stale {
            actual: Some(value),
            ..
        } => Some(value.clone()),
        PfFileState::Missing { .. }
        | PfFileState::Stale { actual: None, .. }
        | PfFileState::Conflict { .. }
        | PfFileState::Unreadable { .. } => None,
    }
}

fn pf_reference_from_state(
    state: &PfFileState<platform::PfConfReference>,
) -> Option<platform::PfConfReference> {
    match state {
        PfFileState::Current { value, .. }
        | PfFileState::Stale {
            actual: Some(value),
            ..
        } => Some(*value),
        PfFileState::Missing { .. }
        | PfFileState::Stale { actual: None, .. }
        | PfFileState::Conflict { .. }
        | PfFileState::Unreadable { .. } => None,
    }
}

fn metadata_from_local_ca_state(state: &CaFileState) -> Option<platform::LocalCaMetadata> {
    match state {
        CaFileState::Current { metadata, .. } => Some(metadata.clone()),
        CaFileState::Missing { .. }
        | CaFileState::RepairRequired { .. }
        | CaFileState::Unreadable { .. } => None,
    }
}

fn system_trust_state(
    environment: &impl Environment,
    metadata: Option<&platform::LocalCaMetadata>,
) -> TrustDomainState {
    struct EnvironmentTrustInspector<'environment, E> {
        environment: &'environment E,
    }

    impl<E: Environment> platform::SystemTrustInspector for EnvironmentTrustInspector<'_, E> {
        fn trusted_certificates(
            &self,
        ) -> Result<Vec<platform::KeychainCertificate>, platform::PlatformError> {
            self.environment.trusted_ca_certificates()
        }
    }

    platform::inspect_system_ca_trust(metadata, &EnvironmentTrustInspector { environment })
}

fn record_default_resource_desired_state(
    paths: &PvPaths,
    default_resource_plan: &[SetupResourcePlan],
) -> Result<(), ExecuteError> {
    let mut database = Database::open(paths)?;

    for planned in default_resource_plan {
        database.record_managed_resource_track_desired(
            planned.resource_name.as_str(),
            planned.track.as_str(),
            ManagedResourceDesiredState::Installed,
        )?;
    }

    Ok(())
}

fn configure_shell_integration(
    args: &SetupArgs,
    environment: &impl Environment,
    paths: &PvPaths,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let mut output = Output::new(stdout, OutputMode::plain());

    if args.no_path {
        output.line("Shell profile integration skipped by --no-path.")?;
        write_manual_shell_integration(&mut output, None)?;

        return Ok(ExitCode::SUCCESS);
    }

    let Some(shell_path) = environment.var_os("SHELL") else {
        output.line("Shell profile integration skipped because $SHELL is not set.")?;
        write_manual_shell_integration(&mut output, None)?;

        return Ok(ExitCode::SUCCESS);
    };
    let Some(shell) = Shell::detect(shell_path.as_os_str()) else {
        output.line(&format!(
            "Shell profile integration skipped for unsupported shell: {}",
            shell_path.to_string_lossy()
        ))?;
        write_manual_shell_integration(&mut output, None)?;

        return Ok(ExitCode::SUCCESS);
    };

    let profile_path = shell_profile_path(paths.home(), shell);
    let block = shell_profile_block(shell);
    let existing = read_user_file(&profile_path)?;
    let (next_content, action) = match existing.as_deref() {
        Some(content) => {
            let transform = remove_pv_env_block(content);
            if !transform.complete {
                output.line(&format!(
                    "Shell profile has an incomplete PV ENV block; leaving it unchanged: {profile_path}"
                ))?;

                return Ok(ExitCode::FAILURE);
            }

            let next = if transform.found {
                append_shell_block(&transform.content, &block)
            } else {
                append_shell_block(content, &block)
            };
            if next == content {
                output.line(&format!(
                    "Shell profile integration already current: {profile_path}"
                ))?;

                return Ok(ExitCode::SUCCESS);
            }

            let action = if transform.found { "repair" } else { "update" };
            (next, action)
        }
        None => (block, "create"),
    };

    if args.non_interactive {
        output.line(&format!(
            "Shell profile integration requires {action}; rerun without --non-interactive or use --no-path: {profile_path}"
        ))?;

        return Ok(ExitCode::FAILURE);
    }

    if !args.yes && !confirm_shell_profile_update(environment, stdout, &profile_path, action)? {
        return Ok(ExitCode::FAILURE);
    }

    if existing.is_some() {
        let backup_path = backup_user_file(&profile_path)?;
        let mut output = Output::new(stdout, OutputMode::plain());
        output.line(&format!("Backed up shell profile: {backup_path}"))?;
    }
    write_user_file(&profile_path, &next_content)?;

    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!(
        "Updated shell profile integration: {profile_path}"
    ))?;
    write_manual_shell_integration(&mut output, Some(shell))?;

    Ok(ExitCode::SUCCESS)
}

fn remove_shell_integration(
    environment: &impl Environment,
    paths: &PvPaths,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let mut output = Output::new(stdout, OutputMode::plain());
    let Some(shell_path) = environment.var_os("SHELL") else {
        output.line("Shell profile integration not inspected because $SHELL is not set.")?;

        return Ok(ExitCode::SUCCESS);
    };
    let Some(shell) = Shell::detect(shell_path.as_os_str()) else {
        output.line(&format!(
            "Shell profile integration not inspected for unsupported shell: {}",
            shell_path.to_string_lossy()
        ))?;

        return Ok(ExitCode::SUCCESS);
    };

    let profile_path = shell_profile_path(paths.home(), shell);
    let Some(content) = read_user_file(&profile_path)? else {
        output.line(&format!("Shell profile already absent: {profile_path}"))?;

        return Ok(ExitCode::SUCCESS);
    };
    let transform = remove_pv_env_block(&content);

    if !transform.complete {
        output.line(&format!(
            "Shell profile has an incomplete PV ENV block; leaving it unchanged: {profile_path}"
        ))?;

        return Ok(ExitCode::FAILURE);
    }
    if !transform.found {
        output.line(&format!(
            "Shell profile has no PV ENV block: {profile_path}"
        ))?;

        return Ok(ExitCode::SUCCESS);
    }

    let backup_path = backup_user_file(&profile_path)?;
    write_user_file(&profile_path, &transform.content)?;
    output.line(&format!("Backed up shell profile: {backup_path}"))?;
    output.line(&format!(
        "Removed shell profile integration: {profile_path}"
    ))?;

    Ok(ExitCode::SUCCESS)
}

fn confirm_shell_profile_update(
    environment: &impl Environment,
    stdout: &mut impl Write,
    profile_path: &Utf8Path,
    action: &str,
) -> Result<bool, ExecuteError> {
    let mut output = Output::new(stdout, OutputMode::plain());

    if !environment.stdin_is_terminal() {
        output.line(&format!(
            "Shell profile integration requires {action}; rerun with --yes or --no-path: {profile_path}"
        ))?;

        return Ok(false);
    }

    output.line(&format!(
        "Update shell profile for PV ENV integration ({action})? {profile_path}"
    ))?;
    output.line("Enter y to continue:")?;
    let answer = environment.read_line()?;

    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn confirm_prune(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<bool, ExecuteError> {
    let mut output = Output::new(stdout, OutputMode::plain());

    if !environment.stdin_is_terminal() {
        output.line("Refusing to prune PV state without an interactive confirmation.")?;
        output
            .line("Rerun with `pv uninstall --prune --force` to remove ~/.pv non-interactively.")?;

        return Ok(false);
    }

    output.line("This will permanently remove all PV-owned state under ~/.pv.")?;
    output.line("Enter y to continue:")?;
    let answer = environment.read_line()?;

    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn untrust_ca_for_uninstall(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let local_state =
        platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());

    if matches!(local_state, CaFileState::Missing { .. }) {
        let fingerprints = trusted_pv_ca_fingerprints(environment)?;
        let mut output = Output::new(stdout, OutputMode::plain());
        if fingerprints.is_empty() {
            output
                .line("PV local CA files are absent; System keychain trust is already absent.")?;

            return Ok(ExitCode::SUCCESS);
        }

        for fingerprint in fingerprints {
            environment.untrust_system_ca(&fingerprint, platform::PrivilegeMode::Interactive)?;
            output.line(&format!(
                "Removed stale PV local CA trust from the System keychain: {fingerprint}"
            ))?;
        }

        return Ok(ExitCode::SUCCESS);
    }

    ca::untrust(environment, stdout)
}

const fn setup_privilege_mode(non_interactive: bool) -> platform::PrivilegeMode {
    if non_interactive {
        platform::PrivilegeMode::NonInteractive
    } else {
        platform::PrivilegeMode::Interactive
    }
}

fn remove_default_state(paths: &PvPaths, stdout: &mut impl Write) -> Result<(), ExecuteError> {
    state::fs::remove_daemon_socket(paths)?;

    let mut output = Output::new(stdout, OutputMode::plain());
    for (label, path) in [
        ("PV app binaries and shims", paths.bin()),
        ("runtime metadata", paths.run()),
        ("generated configs", paths.config()),
        ("download cache", paths.downloads()),
    ] {
        if delete_optional_dir(path)? {
            output.line(&format!("Removed {label}: {path}"))?;
        } else {
            output.line(&format!("{label} already absent: {path}"))?;
        }
    }
    output.line("Preserved logs, pv.db, certificates, Composer home/cache, and resources data.")?;

    Ok(())
}

fn trusted_pv_ca_fingerprints(environment: &impl Environment) -> Result<Vec<String>, ExecuteError> {
    struct EnvironmentTrustInspector<'environment, E> {
        environment: &'environment E,
    }

    impl<E: Environment> platform::SystemTrustInspector for EnvironmentTrustInspector<'_, E> {
        fn trusted_certificates(
            &self,
        ) -> Result<Vec<platform::KeychainCertificate>, platform::PlatformError> {
            self.environment.trusted_ca_certificates()
        }
    }

    Ok(platform::trusted_pv_ca_fingerprints(
        &EnvironmentTrustInspector { environment },
    )?)
}

fn prune_state(paths: &PvPaths, stdout: &mut impl Write) -> Result<(), ExecuteError> {
    state::fs::remove_daemon_socket(paths)?;
    let mut output = Output::new(stdout, OutputMode::plain());

    if delete_optional_dir(paths.root())? {
        output.line(&format!("Removed PV state: {}", paths.root()))?;
    } else {
        output.line(&format!("PV state already absent: {}", paths.root()))?;
    }

    Ok(())
}

fn delete_optional_dir(path: &Utf8Path) -> Result<bool, ExecuteError> {
    match state::fs::delete_dir_all(path) {
        Ok(()) => Ok(true),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

fn shell_profile_path(home: &Utf8Path, shell: Shell) -> Utf8PathBuf {
    match shell {
        Shell::Bash => home.join(".bash_profile"),
        Shell::Fish => home.join(".config/fish/config.fish"),
        Shell::Zsh => home.join(".zprofile"),
    }
}

fn shell_profile_block(shell: Shell) -> String {
    match shell {
        Shell::Bash | Shell::Zsh => format!(
            r#"{PV_ENV_START}
if [ -x "$HOME/.pv/bin/pv" ]; then
  eval "$("$HOME/.pv/bin/pv" env --shell {shell_name})"
fi
{PV_ENV_END}
"#,
            shell_name = shell_name(shell),
        ),
        Shell::Fish => format!(
            r#"{PV_ENV_START}
if test -x "$HOME/.pv/bin/pv"
  eval ("$HOME/.pv/bin/pv" env --shell {shell_name} | string collect)
end
{PV_ENV_END}
"#,
            shell_name = shell_name(shell),
        ),
    }
}

fn write_manual_shell_integration(
    output: &mut Output<'_, impl Write>,
    shell: Option<Shell>,
) -> io::Result<()> {
    match shell {
        Some(shell) => output.line(&format!(
            "Open a new terminal, or run `pv env --shell {}` for current-session shell integration.",
            shell_name(shell)
        )),
        None => output.line(
            "Run `pv env --shell zsh`, `pv env --shell bash`, or `pv env --shell fish` for manual shell integration.",
        ),
    }
}

#[derive(Debug)]
struct BlockTransform {
    content: String,
    found: bool,
    complete: bool,
}

fn remove_pv_env_block(content: &str) -> BlockTransform {
    let mut next = String::new();
    let mut found = false;
    let mut in_block = false;
    let mut complete = true;

    for line in content.lines() {
        match (line.trim(), in_block) {
            (PV_ENV_START, false) => {
                found = true;
                in_block = true;
            }
            (PV_ENV_START, true) => {
                complete = false;
            }
            (PV_ENV_END, true) => {
                in_block = false;
            }
            (_line, true) => {}
            (_line, false) => {
                next.push_str(line);
                next.push('\n');
            }
        }
    }

    if in_block {
        complete = false;
    }

    BlockTransform {
        content: next,
        found,
        complete,
    }
}

fn append_shell_block(content: &str, block: &str) -> String {
    if content.trim().is_empty() {
        return block.to_string();
    }

    let mut next = content.trim_end_matches('\n').to_string();
    next.push_str("\n\n");
    next.push_str(block);

    next
}

fn shell_name(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => "bash",
        Shell::Fish => "fish",
        Shell::Zsh => "zsh",
    }
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

fn current_executable(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.current_exe()?)
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn resolver_test_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.resolver_test_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn pf_anchor_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.pf_anchor_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn pf_conf_path(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.pf_conf_path())
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn backup_user_file(path: &Utf8Path) -> Result<Utf8PathBuf, ExecuteError> {
    let backup_path = backup_path(path);

    copy_user_file(path, &backup_path)?;

    Ok(backup_path)
}

fn backup_path(path: &Utf8Path) -> Utf8PathBuf {
    let file_name = path.file_name().unwrap_or("profile");
    let timestamp = timestamp_suffix();

    path.with_file_name(format!("{file_name}.{timestamp}.pv.bak"))
}

fn timestamp_suffix() -> String {
    let timestamp = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_error) => 0,
    };

    timestamp.to_string()
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI shell integration helper owns user profile reads"
)]
fn read_user_file(path: &Utf8Path) -> Result<Option<String>, ExecuteError> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(path_io_error(path, error).into()),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI shell integration helper owns user profile writes"
)]
fn write_user_file(path: &Utf8Path, content: &str) -> Result<(), ExecuteError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| path_io_error(parent, source))?;
    }
    std::fs::write(path, content).map_err(|source| path_io_error(path, source).into())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI setup helper owns PV command shim writes"
)]
fn write_executable_file(path: &Utf8Path, content: &str) -> Result<(), ExecuteError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| path_io_error(parent, source))?;
    }

    std::fs::write(path, content).map_err(|source| path_io_error(path, source))?;
    set_command_shim_permissions(path)
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "CLI setup helper owns PV command shim permission updates"
)]
fn set_command_shim_permissions(path: &Utf8Path) -> Result<(), ExecuteError> {
    use std::os::unix::fs::PermissionsExt as _;

    let permissions = std::fs::Permissions::from_mode(SHIM_FILE_MODE);
    std::fs::set_permissions(path, permissions).map_err(|source| path_io_error(path, source).into())
}

#[cfg(not(unix))]
fn set_command_shim_permissions(path: &Utf8Path) -> Result<(), ExecuteError> {
    Err(path_io_error(
        path,
        io::Error::new(
            io::ErrorKind::Unsupported,
            "PV command shims require Unix permissions",
        ),
    )
    .into())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI shell integration helper owns user profile backups"
)]
fn copy_user_file(source: &Utf8Path, destination: &Utf8Path) -> Result<(), ExecuteError> {
    std::fs::copy(source, destination)
        .map(|_bytes| ())
        .map_err(|error| path_io_error(destination, error).into())
}

fn path_io_error(path: &Utf8Path, source: io::Error) -> io::Error {
    io::Error::new(source.kind(), format!("{path}: {source}"))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'\''"#))
}
