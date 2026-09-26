use std::io;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use platform::{CaFileState, GeneratedLocalCa, LocalCaMetadata, TrustDomainState};
use state::{PvPaths, StateError};

use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Line, Mark, Output, Streams};

pub(crate) fn status(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let local_state =
        platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let local_metadata = metadata_from_local_state(&local_state);
    let trust_state = trust_state(environment, local_metadata.as_ref());
    let output = &mut streams.out;

    output.heading("ca:status", Some("CA trust status"))?;
    write_local_ca_state(output, &local_state)?;
    write_system_trust_state(output, &trust_state)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn trust(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let initial_state =
        platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let (local_state, generated) = ensure_local_ca(&paths, initial_state)?;
    let local_metadata = metadata_from_local_state(&local_state);
    let trust_state = trust_state(environment, local_metadata.as_ref());
    let output = &mut streams.out;

    output.success("Prepared PV local CA")?;
    match generated {
        Some(generated) => {
            write_ca_paths(output, &paths.ca_certificate(), &paths.ca_private_key())?;
            output.detail(Line::field(
                "fingerprint: ",
                &generated.metadata.fingerprint,
            ))?;
        }
        None => output.detail("existing local CA is current")?,
    }
    write_system_trust_state(output, &trust_state)?;

    match trust_state {
        TrustDomainState::Current { .. } => {
            output.note("System keychain trust already matches PV.")?;
            Ok(ExitCode::SUCCESS)
        }
        TrustDomainState::NotTrusted { .. } => {
            environment.trust_system_ca(&paths.ca_certificate())?;
            output.success("Trusted PV local CA in the System keychain.")?;
            Ok(ExitCode::SUCCESS)
        }
        TrustDomainState::Denied { fingerprint } => {
            environment.untrust_system_ca(&fingerprint)?;
            environment.trust_system_ca(&paths.ca_certificate())?;
            output.success("Removed denied PV local CA trust from the System keychain.")?;
            output.success("Trusted PV local CA in the System keychain.")?;
            Ok(ExitCode::SUCCESS)
        }
        TrustDomainState::Stale {
            actual_fingerprint, ..
        } => {
            environment.untrust_system_ca(&actual_fingerprint)?;
            environment.trust_system_ca(&paths.ca_certificate())?;
            output.success("Removed stale PV local CA trust from the System keychain.")?;
            output.success("Trusted PV local CA in the System keychain.")?;
            Ok(ExitCode::SUCCESS)
        }
        TrustDomainState::Unknown { .. } | TrustDomainState::Unreadable { .. } => {
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(crate) fn untrust(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let local_state =
        platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let local_metadata = metadata_from_local_state(&local_state);
    let trust_state = trust_state(environment, local_metadata.as_ref());
    let output = &mut streams.out;

    output.success("Prepared PV local CA trust removal")?;
    write_local_ca_state(output, &local_state)?;
    write_system_trust_state(output, &trust_state)?;

    match trust_state {
        TrustDomainState::NotTrusted { .. } => {
            output.note("System keychain trust is already absent.")?;
            Ok(ExitCode::SUCCESS)
        }
        TrustDomainState::Current { fingerprint } | TrustDomainState::Denied { fingerprint } => {
            environment.untrust_system_ca(&fingerprint)?;
            output.success("Removed PV local CA trust from the System keychain.")?;
            Ok(ExitCode::SUCCESS)
        }
        TrustDomainState::Stale {
            actual_fingerprint, ..
        } => {
            environment.untrust_system_ca(&actual_fingerprint)?;
            output.success("Removed stale PV local CA trust from the System keychain.")?;
            Ok(ExitCode::SUCCESS)
        }
        TrustDomainState::Unknown { .. } | TrustDomainState::Unreadable { .. } => {
            Ok(ExitCode::FAILURE)
        }
    }
}

fn ensure_local_ca(
    paths: &PvPaths,
    initial_state: CaFileState,
) -> Result<(CaFileState, Option<GeneratedLocalCa>), ExecuteError> {
    ensure_local_ca_with_generator(paths, initial_state, platform::generate_local_ca)
}

fn ensure_local_ca_with_generator(
    paths: &PvPaths,
    initial_state: CaFileState,
    generate: impl FnOnce() -> Result<GeneratedLocalCa, platform::PlatformError>,
) -> Result<(CaFileState, Option<GeneratedLocalCa>), ExecuteError> {
    if matches!(initial_state, CaFileState::Current { .. }) {
        return Ok((initial_state, None));
    }

    let generated = generate()?;
    state::fs::write_sensitive_file(&paths.ca_certificate(), &generated.certificate_pem)?;
    state::fs::write_sensitive_file(&paths.ca_private_key(), &generated.private_key_pem)?;
    let repaired_state =
        platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());

    match &repaired_state {
        CaFileState::Current { .. } => Ok((repaired_state, Some(generated))),
        CaFileState::Missing { .. } => Err(platform::PlatformError::LocalCaPostWriteMissing.into()),
        CaFileState::RepairRequired { reason, .. } => {
            Err(platform::PlatformError::LocalCaPostWriteRepairRequired { reason: *reason }.into())
        }
        CaFileState::Unreadable { path, message } => {
            Err(platform::PlatformError::LocalCaPostWriteUnreadable {
                path: path.clone(),
                message: message.clone(),
            }
            .into())
        }
    }
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

    impl<E: Environment> platform::SystemTrustInspector for EnvironmentTrustInspector<'_, E> {
        fn trusted_certificates(
            &self,
        ) -> Result<Vec<platform::KeychainCertificate>, platform::PlatformError> {
            self.environment.trusted_ca_certificates()
        }
    }

    let inspector = EnvironmentTrustInspector { environment };
    platform::inspect_system_ca_trust(metadata, &inspector)
}

fn write_local_ca_state(output: &mut Output<'_>, state: &CaFileState) -> io::Result<()> {
    match state {
        CaFileState::Missing {
            certificate_path,
            private_key_path,
        } => {
            output.note("Local CA files: missing")?;
            write_ca_paths(output, certificate_path, private_key_path)
        }
        CaFileState::Current {
            certificate_path,
            private_key_path,
            metadata,
        } => {
            output.success("Local CA files: current")?;
            write_ca_paths(output, certificate_path, private_key_path)?;
            output.detail(Line::field("common name: ", &metadata.common_name))?;
            output.detail(Line::field("fingerprint: ", &metadata.fingerprint))
        }
        CaFileState::RepairRequired {
            certificate_path,
            private_key_path,
            reason,
        } => {
            output.failure("Local CA files: repair required")?;
            write_ca_paths(output, certificate_path, private_key_path)?;
            output.detail(format!("reason: {reason:?}"))
        }
        CaFileState::Unreadable { path, message } => {
            output.failure("Local CA files: unreadable")?;
            output.detail(Line::field("path: ", path))?;
            output.detail(message)
        }
    }
}

fn write_ca_paths(
    output: &mut Output<'_>,
    certificate_path: &Utf8Path,
    private_key_path: &Utf8Path,
) -> io::Result<()> {
    output.detail(Line::field("certificate: ", certificate_path))?;
    output.detail(Line::field("private key: ", private_key_path))
}

fn write_system_trust_state(output: &mut Output<'_>, state: &TrustDomainState) -> io::Result<()> {
    let (mark, status) = match state {
        TrustDomainState::Current { .. } => (Mark::Success, "current"),
        TrustDomainState::NotTrusted { .. } => (Mark::Warning, "not trusted"),
        TrustDomainState::Stale { .. } => (Mark::Warning, "stale"),
        TrustDomainState::Denied { .. } => (Mark::Failure, "denied"),
        TrustDomainState::Unknown { .. } => (Mark::Warning, "unknown"),
        TrustDomainState::Unreadable { .. } => (Mark::Failure, "unreadable"),
    };
    output.status(mark, format!("System keychain trust: {status}"))?;
    match state {
        TrustDomainState::Current { fingerprint }
        | TrustDomainState::NotTrusted { fingerprint }
        | TrustDomainState::Denied { fingerprint } => {
            output.detail(Line::field("fingerprint: ", fingerprint))
        }
        TrustDomainState::Stale {
            expected_fingerprint,
            actual_fingerprint,
        } => {
            output.detail(Line::field("expected fingerprint: ", expected_fingerprint))?;
            output.detail(Line::field("actual fingerprint: ", actual_fingerprint))
        }
        TrustDomainState::Unknown { reason } => output.detail(reason.to_string()),
        TrustDomainState::Unreadable { message } => output.detail(message),
    }
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

#[cfg(test)]
mod tests {
    use camino_tempfile::tempdir;
    use platform::{CaRepairReason, PlatformError};

    use super::*;

    #[test]
    fn ensure_local_ca_rejects_failed_post_write_validation() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let generated = platform::generate_local_ca()?;
        let mut invalid_generated = generated.clone();
        invalid_generated.private_key_pem = "not a private key\n".to_string();
        let initial_state = CaFileState::Missing {
            certificate_path: paths.ca_certificate(),
            private_key_path: paths.ca_private_key(),
        };

        let result =
            ensure_local_ca_with_generator(&paths, initial_state, || Ok(invalid_generated));

        assert!(matches!(
            result,
            Err(ExecuteError::Platform(
                PlatformError::LocalCaPostWriteRepairRequired {
                    reason: CaRepairReason::MalformedPrivateKey
                }
            ))
        ));

        Ok(())
    }
}
