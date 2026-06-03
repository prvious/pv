use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use macos::{CaFileState, GeneratedLocalCa, LocalCaMetadata, TrustDomainState};
use state::{PvPaths, StateError};

use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Output, OutputMode};

pub(crate) fn status(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let local_state =
        macos::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let local_metadata = metadata_from_local_state(&local_state);
    let trust_state = trust_state(environment, local_metadata.as_ref());
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("CA trust status")?;
    write_local_ca_state(&mut output, &local_state)?;
    write_system_trust_state(&mut output, &trust_state)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn trust(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let initial_state =
        macos::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let (local_state, generated) = ensure_local_ca(&paths, initial_state)?;
    let local_metadata = metadata_from_local_state(&local_state);
    let trust_state = trust_state(environment, local_metadata.as_ref());
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("Prepared PV local CA")?;
    match generated {
        Some(generated) => {
            output.line(&format!("  certificate: {}", paths.ca_certificate()))?;
            output.line(&format!("  private key: {}", paths.ca_private_key()))?;
            output.line(&format!(
                "  fingerprint: {}",
                generated.metadata.fingerprint
            ))?;
        }
        None => output.line("  existing local CA is current")?,
    }
    write_system_trust_state(&mut output, &trust_state)?;
    output
        .line("Privileged trust installation deferred to PR 13 setup/system-integration work.")?;

    Ok(ExitCode::FAILURE)
}

pub(crate) fn untrust(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let local_state =
        macos::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let local_metadata = metadata_from_local_state(&local_state);
    let trust_state = trust_state(environment, local_metadata.as_ref());
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line("Prepared PV local CA trust removal")?;
    write_local_ca_state(&mut output, &local_state)?;
    write_system_trust_state(&mut output, &trust_state)?;

    match trust_state {
        TrustDomainState::NotTrusted { .. } => {
            output.line("System keychain trust is already absent.")?;
            Ok(ExitCode::SUCCESS)
        }
        TrustDomainState::Current { .. } | TrustDomainState::Stale { .. } => {
            output.line(
                "Privileged trust removal deferred to PR 13 setup/system-integration work.",
            )?;
            Ok(ExitCode::FAILURE)
        }
        TrustDomainState::Denied { .. }
        | TrustDomainState::Unknown { .. }
        | TrustDomainState::Unreadable { .. } => Ok(ExitCode::FAILURE),
    }
}

fn ensure_local_ca(
    paths: &PvPaths,
    initial_state: CaFileState,
) -> Result<(CaFileState, Option<GeneratedLocalCa>), ExecuteError> {
    if matches!(initial_state, CaFileState::Current { .. }) {
        return Ok((initial_state, None));
    }

    let generated = macos::generate_local_ca()?;
    state::fs::write_sensitive_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    state::fs::write_sensitive_file(&paths.ca_private_key(), &generated.private_key_pem)?;
    let repaired_state =
        macos::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());

    Ok((repaired_state, Some(generated)))
}

fn metadata_from_local_state(state: &CaFileState) -> Option<LocalCaMetadata> {
    match state {
        CaFileState::Current { metadata, .. } => Some(metadata.clone()),
        CaFileState::Missing { .. }
        | CaFileState::RepairRequired { .. }
        | CaFileState::Unreadable { .. } => None,
    }
}

fn trust_state(
    environment: &impl Environment,
    metadata: Option<&LocalCaMetadata>,
) -> TrustDomainState {
    struct EnvironmentTrustInspector<'environment, E> {
        environment: &'environment E,
    }

    impl<E: Environment> macos::SystemTrustInspector for EnvironmentTrustInspector<'_, E> {
        fn trusted_certificates(
            &self,
        ) -> Result<Vec<macos::KeychainCertificate>, macos::MacosError> {
            self.environment.trusted_ca_certificates()
        }
    }

    let inspector = EnvironmentTrustInspector { environment };
    macos::inspect_system_ca_trust(metadata, &inspector)
}

fn write_local_ca_state(
    output: &mut Output<'_, impl Write>,
    state: &CaFileState,
) -> io::Result<()> {
    match state {
        CaFileState::Missing {
            certificate_path,
            private_key_path,
        } => {
            output.line("Local CA files: missing")?;
            output.line(&format!("  certificate: {certificate_path}"))?;
            output.line(&format!("  private key: {private_key_path}"))
        }
        CaFileState::Current {
            certificate_path,
            private_key_path,
            metadata,
        } => {
            output.line("Local CA files: current")?;
            output.line(&format!("  certificate: {certificate_path}"))?;
            output.line(&format!("  private key: {private_key_path}"))?;
            output.line(&format!("  common name: {}", metadata.common_name))?;
            output.line(&format!("  fingerprint: {}", metadata.fingerprint))
        }
        CaFileState::RepairRequired {
            certificate_path,
            private_key_path,
            reason,
        } => {
            output.line("Local CA files: repair required")?;
            output.line(&format!("  certificate: {certificate_path}"))?;
            output.line(&format!("  private key: {private_key_path}"))?;
            output.line(&format!("  reason: {reason:?}"))
        }
        CaFileState::Unreadable { path, message } => {
            output.line("Local CA files: unreadable")?;
            output.line(&format!("  path: {path}"))?;
            output.line(&format!("  {message}"))
        }
    }
}

fn write_system_trust_state(
    output: &mut Output<'_, impl Write>,
    state: &TrustDomainState,
) -> io::Result<()> {
    match state {
        TrustDomainState::Current { fingerprint } => {
            output.line("System keychain trust: current")?;
            output.line(&format!("  fingerprint: {fingerprint}"))
        }
        TrustDomainState::NotTrusted { fingerprint } => {
            output.line("System keychain trust: not trusted")?;
            output.line(&format!("  fingerprint: {fingerprint}"))
        }
        TrustDomainState::Stale {
            expected_fingerprint,
            actual_fingerprint,
        } => {
            output.line("System keychain trust: stale")?;
            output.line(&format!("  expected fingerprint: {expected_fingerprint}"))?;
            output.line(&format!("  actual fingerprint: {actual_fingerprint}"))
        }
        TrustDomainState::Denied { fingerprint } => {
            output.line("System keychain trust: denied")?;
            output.line(&format!("  fingerprint: {fingerprint}"))
        }
        TrustDomainState::Unknown { reason } => {
            output.line("System keychain trust: unknown")?;
            output.line(&format!("  {reason}"))
        }
        TrustDomainState::Unreadable { message } => {
            output.line("System keychain trust: unreadable")?;
            output.line(&format!("  {message}"))
        }
    }
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}
