use std::io;
use std::io::Write;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};
use platform::CaFileState;
use resources::{ArtifactManifestCache, ResourceName, TrackName, TrackSelector};
use state::{Database, ManagedResourceDesiredState, PvPaths, StateError};

use super::{ca, daemon as daemon_command, dns, ports};
use crate::args::{SetupArgs, UninstallArgs};
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};
use crate::shell::Shell;

const PV_ENV_START: &str = "# >>> PV ENV";
const PV_ENV_END: &str = "# <<< PV ENV";

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

    if configure_shell_integration(&args, environment, &paths, stdout)? != ExitCode::SUCCESS {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("DNS resolver setup", stdout, |stdout| {
        dns::install(environment, stdout)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("port redirect setup", stdout, |stdout| {
        ports::install(environment, stdout)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    if !run_required_step("CA trust setup", stdout, |stdout| {
        ca::trust(environment, stdout)
    })? {
        return Ok(ExitCode::FAILURE);
    }
    record_default_resource_desired_state(&paths)?;
    if !run_required_step("daemon registration", stdout, |stdout| {
        daemon_command::enable(environment, stdout)
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

fn record_default_resource_desired_state(paths: &PvPaths) -> Result<(), ExecuteError> {
    let manifest_cache = ArtifactManifestCache::new(paths.downloads());
    let manifest = match manifest_cache.load_cached() {
        Ok(manifest) => manifest,
        Err(resources::ResourcesError::Filesystem { .. }) => {
            return Err(CliError::MissingSetupArtifactManifest {
                path: manifest_cache.path().to_string(),
            }
            .into());
        }
        Err(error) => return Err(error.into()),
    };
    let mut database = Database::open(paths)?;

    for resource_default in DEFAULT_SETUP_RESOURCES {
        let resource_name = ResourceName::new(resource_default.resource_name)?;
        let track_selector = match resource_default.track {
            SetupResourceTrackDefault::ManifestDefault => TrackSelector::Latest,
            SetupResourceTrackDefault::Concrete(track) => {
                TrackSelector::Track(TrackName::new(track)?)
            }
        };
        let track = manifest.resolve_track(&resource_name, track_selector)?;

        database.record_managed_resource_track_desired(
            resource_name.as_str(),
            track.as_str(),
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

    if args.non_interactive && !args.yes {
        output.line(&format!(
            "Shell profile integration requires {action}; rerun with --yes or --no-path: {profile_path}"
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
        let mut output = Output::new(stdout, OutputMode::plain());
        output.line("PV local CA files are absent; skipping System keychain trust removal.")?;

        return Ok(ExitCode::SUCCESS);
    }

    ca::untrust(environment, stdout)
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
