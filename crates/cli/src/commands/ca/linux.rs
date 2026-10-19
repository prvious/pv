use std::collections::BTreeSet;
use std::process::ExitCode;

use platform::TrustDomainState;

use super::{
    BrowserCaReport, browser_ca_reports, ensure_local_ca, metadata_from_local_state, pv_paths,
    trust_state, write_ca_paths, write_local_ca_state,
};
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Line, Mark, Output, Streams};

pub(super) fn status(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let local = platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let metadata = metadata_from_local_state(&local);
    let system = trust_state(environment, metadata.as_ref());
    streams.out.heading("ca:status", Some("CA trust status"))?;
    write_local_ca_state(&mut streams.out, &local)?;
    write_system_state(&mut streams.out, &system)?;
    for report in browser_ca_reports(environment, &paths, metadata.as_ref())? {
        write_browser_state(&mut streams.out, &report)?;
    }
    Ok(ExitCode::SUCCESS)
}

pub(super) fn trust(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let initial =
        platform::inspect_local_ca_files(&paths.ca_certificate(), &paths.ca_private_key());
    let (local, generated) = ensure_local_ca(&paths, initial)?;
    let metadata = metadata_from_local_state(&local);
    let system = trust_state(environment, metadata.as_ref());
    streams.out.success("Prepared PV local CA")?;
    if generated.is_some() {
        write_ca_paths(
            &mut streams.out,
            &paths.ca_certificate(),
            &paths.ca_private_key(),
        )?;
    }
    match &system {
        TrustDomainState::Current { .. } => {}
        TrustDomainState::NotTrusted { .. } => {
            environment.trust_system_ca(&paths.ca_certificate())?
        }
        TrustDomainState::Denied { fingerprint }
        | TrustDomainState::Stale {
            actual_fingerprint: fingerprint,
            ..
        } => {
            environment.untrust_system_ca(fingerprint)?;
            environment.trust_system_ca(&paths.ca_certificate())?;
        }
        TrustDomainState::Unknown { .. } | TrustDomainState::Unreadable { .. } => {
            write_system_state(&mut streams.out, &system)?;
            return Ok(ExitCode::FAILURE);
        }
    }
    streams
        .out
        .success("PV local CA is trusted in the system store")?;
    for database in environment.browser_ca_databases(paths.home())? {
        environment.trust_browser_ca(&database, &paths.ca_certificate())?;
        let report = BrowserCaReport {
            state: environment.inspect_browser_ca_trust(&database, metadata.as_ref()),
            database,
        };
        write_browser_state(&mut streams.out, &report)?;
        if !report.current() {
            return Ok(ExitCode::FAILURE);
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub(super) fn untrust(
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    for database in environment.browser_ca_databases(paths.home())? {
        environment.untrust_browser_ca(&database)?;
        streams
            .out
            .success(Line::field("Removed PV browser CA trust: ", &database))?;
    }
    let fingerprints = environment
        .trusted_ca_certificates()?
        .into_iter()
        .map(|certificate| certificate.metadata.fingerprint)
        .collect::<BTreeSet<_>>();
    for fingerprint in fingerprints {
        environment.untrust_system_ca(&fingerprint)?;
    }
    streams
        .out
        .success("PV local CA trust is absent from the system and browser stores")?;
    Ok(ExitCode::SUCCESS)
}

pub(super) fn write_browser_state(
    output: &mut Output<'_>,
    report: &BrowserCaReport,
) -> std::io::Result<()> {
    output.status(
        if report.current() {
            Mark::Success
        } else {
            Mark::Failure
        },
        Line::field("Browser CA trust: ", &report.database).text(format!(" · {}", report.label())),
    )?;
    match &report.state {
        TrustDomainState::Unknown { reason } => output.detail(reason),
        TrustDomainState::Unreadable { message } => output.detail(message),
        _ => Ok(()),
    }
}

fn write_system_state(output: &mut Output<'_>, state: &TrustDomainState) -> std::io::Result<()> {
    let (mark, label) = match state {
        TrustDomainState::Current { .. } => (Mark::Success, "current"),
        TrustDomainState::NotTrusted { .. } => (Mark::Warning, "not trusted"),
        TrustDomainState::Stale { .. } => (Mark::Warning, "stale"),
        TrustDomainState::Denied { .. } => (Mark::Warning, "denied"),
        TrustDomainState::Unknown { .. } => (Mark::Failure, "unknown"),
        TrustDomainState::Unreadable { .. } => (Mark::Failure, "unreadable"),
    };
    output.status(mark, format!("System CA trust: {label}"))?;
    match state {
        TrustDomainState::Unknown { reason } => output.detail(reason),
        TrustDomainState::Unreadable { message } => output.detail(message),
        _ => Ok(()),
    }
}
