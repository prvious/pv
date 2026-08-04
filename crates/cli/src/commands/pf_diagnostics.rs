use camino::Utf8PathBuf;
use platform::{ActivePfRedirectInspection, PfConfReference, PfFileState, PfRedirectConfig};
use serde::Serialize;
use state::{Database, GatewayPort, PortOwner, PvPaths};
use time::OffsetDateTime;

use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum PfRoutingState {
    Active,
    Inactive,
    Drifted,
    Unknown,
}

impl PfRoutingState {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
            Self::Drifted => "drifted",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum PfRoutingEvidence {
    Pfctl,
    Probe,
    Unavailable,
}

impl PfRoutingEvidence {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Pfctl => "pfctl",
            Self::Probe => "probe",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct PfRoutingDiagnostic {
    pub(super) state: PfRoutingState,
    pub(super) evidence: PfRoutingEvidence,
    pub(super) expected_http_port: Option<u16>,
    pub(super) expected_https_port: Option<u16>,
    pub(super) active_http_port: Option<u16>,
    pub(super) active_https_port: Option<u16>,
    pub(super) observed_at: String,
}

impl PfRoutingDiagnostic {
    pub(super) fn read(
        environment: &impl Environment,
        paths: &PvPaths,
        database: Option<&Database>,
    ) -> Result<Self, ExecuteError> {
        let expected = expected_redirect_config(database)?;
        let prepared_anchor =
            platform::inspect_pf_anchor_file(&paths.pf_anchor_config(), expected.as_ref());
        let expected_reference = PfConfReference;
        let prepared_reference = platform::inspect_pf_conf_reference(
            &paths.pf_conf_reference_config(),
            Some(&expected_reference),
        );
        let system_anchor_path = utf8_path(environment.pf_anchor_path())?;
        let system_reference_path = utf8_path(environment.pf_conf_path())?;
        let system_anchor =
            platform::inspect_pf_anchor_file(&system_anchor_path, expected.as_ref());
        let system_reference =
            platform::inspect_pf_conf_reference(&system_reference_path, Some(&expected_reference));
        let files_current = matches!(prepared_anchor, PfFileState::Current { .. })
            && matches!(prepared_reference, PfFileState::Current { .. })
            && matches!(system_anchor, PfFileState::Current { .. })
            && matches!(system_reference, PfFileState::Current { .. });

        let (state, evidence, active_http_port, active_https_port) =
            match environment.inspect_active_pf_redirects_unprivileged() {
                Ok(inspection) => classify_pfctl(expected.as_ref(), files_current, &inspection),
                Err(_) => classify_probe(environment, paths, expected.as_ref(), files_current),
            };

        Ok(Self {
            state,
            evidence,
            expected_http_port: expected.as_ref().map(|config| config.http_port),
            expected_https_port: expected.as_ref().map(|config| config.https_port),
            active_http_port,
            active_https_port,
            observed_at: timestamp(),
        })
    }

    pub(super) const fn is_active(&self) -> bool {
        matches!(self.state, PfRoutingState::Active)
    }
}

fn classify_pfctl(
    expected: Option<&PfRedirectConfig>,
    files_current: bool,
    inspection: &ActivePfRedirectInspection,
) -> (PfRoutingState, PfRoutingEvidence, Option<u16>, Option<u16>) {
    let active_http_port = inspection.pv_config.as_ref().map(|config| config.http_port);
    let active_https_port = inspection
        .pv_config
        .as_ref()
        .map(|config| config.https_port);

    if inspection.pv_config.as_ref() == expected && expected.is_some() {
        let state = if files_current {
            PfRoutingState::Active
        } else {
            PfRoutingState::Drifted
        };

        return (
            state,
            PfRoutingEvidence::Pfctl,
            active_http_port,
            active_https_port,
        );
    }

    if inspection.pv_config.is_some() {
        return (
            PfRoutingState::Drifted,
            PfRoutingEvidence::Pfctl,
            active_http_port,
            active_https_port,
        );
    }

    let Some(expected) = expected else {
        return (
            PfRoutingState::Inactive,
            PfRoutingEvidence::Pfctl,
            None,
            None,
        );
    };
    let active_http_port = inspection
        .loopback_target_ports
        .contains(&expected.http_port)
        .then_some(expected.http_port);
    let active_https_port = inspection
        .loopback_target_ports
        .contains(&expected.https_port)
        .then_some(expected.https_port);
    let state = if active_http_port.is_some() || active_https_port.is_some() {
        PfRoutingState::Drifted
    } else {
        PfRoutingState::Inactive
    };

    (
        state,
        PfRoutingEvidence::Pfctl,
        active_http_port,
        active_https_port,
    )
}

fn classify_probe(
    environment: &impl Environment,
    paths: &PvPaths,
    expected: Option<&PfRedirectConfig>,
    files_current: bool,
) -> (PfRoutingState, PfRoutingEvidence, Option<u16>, Option<u16>) {
    if let Some(expected) = expected
        && environment
            .probe_gateway_redirects(expected, &paths.ca_certificate())
            .is_ok()
    {
        let state = if files_current {
            PfRoutingState::Active
        } else {
            PfRoutingState::Drifted
        };

        return (
            state,
            PfRoutingEvidence::Probe,
            Some(expected.http_port),
            Some(expected.https_port),
        );
    }

    let state = if files_current {
        PfRoutingState::Unknown
    } else {
        PfRoutingState::Drifted
    };

    (state, PfRoutingEvidence::Unavailable, None, None)
}

fn expected_redirect_config(
    database: Option<&Database>,
) -> Result<Option<PfRedirectConfig>, ExecuteError> {
    let Some(database) = database else {
        return Ok(None);
    };
    let assignments = database.assigned_ports()?;
    let http_port = assignments.iter().find_map(|assignment| {
        (assignment.owner == PortOwner::Gateway(GatewayPort::Http)).then_some(assignment.port)
    });
    let https_port = assignments.iter().find_map(|assignment| {
        (assignment.owner == PortOwner::Gateway(GatewayPort::Https)).then_some(assignment.port)
    });

    Ok(http_port
        .zip(https_port)
        .map(|(http_port, https_port)| PfRedirectConfig::new(http_port, https_port)))
}

fn utf8_path(path: std::path::PathBuf) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(path).map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn timestamp() -> String {
    let now = OffsetDateTime::now_utc();
    let month: u8 = now.month().into();

    format!(
        "{:04}-{month:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn readable_rules_classify_exact_absent_and_partial_redirects() {
        let expected = PfRedirectConfig::new(48080, 48443);
        let exact = ActivePfRedirectInspection {
            pv_config: Some(expected.clone()),
            loopback_target_ports: BTreeSet::from([48080, 48443]),
        };
        let absent = ActivePfRedirectInspection {
            pv_config: None,
            loopback_target_ports: BTreeSet::new(),
        };
        let partial = ActivePfRedirectInspection {
            pv_config: None,
            loopback_target_ports: BTreeSet::from([48080]),
        };

        assert_eq!(
            classify_pfctl(Some(&expected), true, &exact).0,
            PfRoutingState::Active
        );
        assert_eq!(
            classify_pfctl(Some(&expected), true, &absent).0,
            PfRoutingState::Inactive
        );
        assert_eq!(
            classify_pfctl(Some(&expected), true, &partial).0,
            PfRoutingState::Drifted
        );
    }

    #[test]
    fn exact_rules_with_stale_files_are_drifted() {
        let expected = PfRedirectConfig::new(48080, 48443);
        let inspection = ActivePfRedirectInspection {
            pv_config: Some(expected.clone()),
            loopback_target_ports: BTreeSet::from([48080, 48443]),
        };

        assert_eq!(
            classify_pfctl(Some(&expected), false, &inspection).0,
            PfRoutingState::Drifted
        );
    }
}
