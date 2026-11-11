// This is probably temporary. We should remember to remove it on the next release.
//! Removes the pf port redirect that PV versions before launchd sockets installed on macOS. Left
//! in place, pf would keep sending loopback port 80 to a port nothing listens on.

use std::io;

use camino::Utf8Path;

use crate::PlatformError;

// This is probably temporary. We should remember to remove it on the next release.
#[cfg(target_os = "macos")]
const ANCHOR_PATH: &str = "/etc/pf.anchors/com.prvious.pv";
// This is probably temporary. We should remember to remove it on the next release.
#[cfg(target_os = "macos")]
const PF_CONF_PATH: &str = "/etc/pf.conf";
// This is probably temporary. We should remember to remove it on the next release.
const PV_MARKER: &str = "# Managed by PV";
// This is probably temporary. We should remember to remove it on the next release.
/// The lines older PV versions added to `/etc/pf.conf`.
const PV_PF_CONF_LINES: [&str; 5] = [
    PV_MARKER,
    "# Source: PV prepared pf.conf reference for /etc/pf.conf",
    "rdr-anchor \"com.prvious.pv\"",
    "anchor \"com.prvious.pv\"",
    "load anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"",
];

// This is probably temporary. We should remember to remove it on the next release.
/// Whether an older PV left its pf redirect behind. Both files are world-readable.
#[cfg(target_os = "macos")]
pub fn legacy_pf_present() -> Result<bool, PlatformError> {
    legacy_pf_present_at(Utf8Path::new(ANCHOR_PATH), Utf8Path::new(PF_CONF_PATH))
}

// This is probably temporary. We should remember to remove it on the next release.
fn legacy_pf_present_at(anchor: &Utf8Path, pf_conf: &Utf8Path) -> Result<bool, PlatformError> {
    let anchor_present = state::fs::path_entry_exists(anchor)
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;

    Ok(anchor_present
        || read_optional(pf_conf)?
            .is_some_and(|content| content.lines().any(|line| line.trim() == PV_MARKER)))
}

// This is probably temporary. We should remember to remove it on the next release.
#[cfg(target_os = "macos")]
pub(crate) fn remove_legacy_pf_privileged() -> Result<(), PlatformError> {
    let anchor = Utf8Path::new(ANCHOR_PATH);
    let pf_conf = Utf8Path::new(PF_CONF_PATH);
    crate::helper::validate_root_owned_file_if_present(anchor)?;
    crate::helper::validate_root_owned_file_if_present(pf_conf)?;
    remove_legacy_pf_with_runner(
        anchor,
        pf_conf,
        Utf8Path::new("/Library/Application Support/PV/pf"),
        &mut crate::command::run_system_command,
    )
}

// This is probably temporary. We should remember to remove it on the next release.
/// Drops PV's lines from `pf.conf` after `pfctl` accepts the result, deletes PV's anchor, and
/// reloads pf.
#[cfg(any(target_os = "macos", test))]
fn remove_legacy_pf_with_runner(
    anchor: &Utf8Path,
    pf_conf: &Utf8Path,
    candidate_directory: &Utf8Path,
    run_system: &mut impl FnMut(&str, &[&str]) -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    if let Some(current) = read_optional(pf_conf)?
        && current
            .lines()
            .any(|line| PV_PF_CONF_LINES.contains(&line.trim()))
    {
        let candidate = current
            .lines()
            .filter(|line| !PV_PF_CONF_LINES.contains(&line.trim()))
            .map(|line| format!("{line}\n"))
            .collect::<String>();
        let candidate_path = candidate_directory.join("pf.conf.candidate");
        state::fs::write_sensitive_file(&candidate_path, &candidate)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
        let installed =
            run_system("/sbin/pfctl", &["-nf", candidate_path.as_str()]).and_then(|()| {
                run_system(
                    "/usr/bin/install",
                    &["-m", "0644", candidate_path.as_str(), pf_conf.as_str()],
                )
            });
        let cleaned = state::fs::delete_file(&candidate_path)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()));
        installed.and(cleaned)?;
    }
    run_system("/bin/rm", &["-f", anchor.as_str()])?;

    run_system("/sbin/pfctl", &["-f", pf_conf.as_str()])
}

// This is probably temporary. We should remember to remove it on the next release.
fn read_optional(path: &Utf8Path) -> Result<Option<String>, PlatformError> {
    match state::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            Ok(None)
        }
        Err(error) => Err(PlatformError::SystemIntegration(error.to_string())),
    }
}

// This is probably temporary. We should remember to remove it on the next release.
#[cfg(test)]
mod tests {
    use camino_tempfile::tempdir;

    use super::{legacy_pf_present_at, remove_legacy_pf_with_runner};

    // This is probably temporary. We should remember to remove it on the next release.
    #[test]
    fn removal_validates_pf_conf_without_pvs_lines_then_deletes_the_anchor() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let candidate_directory = tempdir.path().join("candidates");
        let anchor = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
        let pf_conf = tempdir.path().join("etc/pf.conf");
        state::fs::write_sensitive_file(
            &anchor,
            "# Managed by PV\nrdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port 48080\n",
        )?;
        state::fs::write_sensitive_file(
            &pf_conf,
            "set skip on lo0\n# Managed by PV\n# Source: PV prepared pf.conf reference for /etc/pf.conf\nrdr-anchor \"com.prvious.pv\"\nload anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"\n",
        )?;
        let present = legacy_pf_present_at(&anchor, &pf_conf)?;
        let mut commands = Vec::new();
        let mut validated_candidate = None;

        remove_legacy_pf_with_runner(
            &anchor,
            &pf_conf,
            &candidate_directory,
            &mut |program, arguments| {
                commands.push(format!("{program} {}", arguments.join(" ")));
                if let ["-nf", path] = arguments {
                    validated_candidate = state::fs::read_to_string(camino::Utf8Path::new(path))
                        .map_err(|error| crate::PlatformError::SystemIntegration(error.to_string()))
                        .map(Some)?;
                }
                Ok(())
            },
        )?;

        assert!(present);
        assert_eq!(validated_candidate.as_deref(), Some("set skip on lo0\n"));
        assert!(!state::fs::path_entry_exists(
            &candidate_directory.join("pf.conf.candidate")
        )?);
        insta::assert_debug_snapshot!(
            commands
                .iter()
                .map(|command| command.replace(tempdir.path().as_str(), "<tempdir>"))
                .collect::<Vec<_>>()
        );

        Ok(())
    }

    // This is probably temporary. We should remember to remove it on the next release.
    #[test]
    fn a_pf_conf_without_pvs_lines_is_left_alone() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let anchor = tempdir.path().join("etc/pf.anchors/com.prvious.pv");
        let pf_conf = tempdir.path().join("etc/pf.conf");
        state::fs::write_sensitive_file(&pf_conf, "set skip on lo0\n")?;

        assert!(!legacy_pf_present_at(&anchor, &pf_conf)?);

        Ok(())
    }
}
