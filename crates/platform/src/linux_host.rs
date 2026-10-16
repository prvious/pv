use procfs::KernelVersion;
use procfs::process::Process;
use rustix::fs::{Access, FileType, access, stat};

use crate::PlatformError;
use crate::daemon_registration::check_user_manager;
use crate::helper::SUDO_EXECUTABLE;

/// Checks PID 1, the kernel, sudo and user-manager requirements for Linux setup.
pub fn check_linux_setup_host(session_address: &str) -> Result<(), PlatformError> {
    let init = Process::new(1)
        .and_then(|process| process.stat())
        .map_err(|error| {
            PlatformError::SystemIntegration(format!(
                "could not inspect PID 1: {error}; make /proc accessible before running `pv setup`"
            ))
        })?;
    let kernel = KernelVersion::current().map_err(|error| {
        PlatformError::SystemIntegration(format!(
            "could not inspect the Linux kernel: {error}; make /proc accessible before running `pv setup`"
        ))
    })?;
    validate_host(&init.comm, kernel)?;
    require_executable(SUDO_EXECUTABLE).map_err(|error| {
        PlatformError::SystemIntegration(format!(
            "{error}; install the sudo package before running `pv setup`"
        ))
    })?;
    check_user_manager(session_address).map_err(|error| {
        PlatformError::SystemIntegration(format!(
            "{error}; run `pv setup` from a logged-in systemd user session"
        ))
    })
}

fn validate_host(init: &str, kernel: KernelVersion) -> Result<(), PlatformError> {
    if init != "systemd" {
        return Err(PlatformError::SystemIntegration(
            "PV requires systemd as PID 1 on Linux; run `pv setup` on a systemd host".to_owned(),
        ));
    }
    if kernel < KernelVersion::new(5, 15, 0) {
        return Err(PlatformError::SystemIntegration(format!(
            "PV requires Linux kernel 5.15 or newer, found {}.{}.{}; upgrade the kernel before running `pv setup`",
            kernel.major, kernel.minor, kernel.patch
        )));
    }
    Ok(())
}

fn require_executable(path: &str) -> Result<(), PlatformError> {
    let metadata = stat(path).map_err(|error| {
        PlatformError::SystemIntegration(format!("could not inspect executable {path}: {error}"))
    })?;
    if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile {
        return Err(PlatformError::SystemIntegration(format!(
            "required executable {path} is not a regular file"
        )));
    }
    access(path, Access::EXEC_OK).map_err(|error| {
        PlatformError::SystemIntegration(format!("could not execute {path}: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use camino_tempfile::tempdir;
    use insta::assert_debug_snapshot;
    use procfs::KernelVersion;
    use rustix::fs::{Mode, chmod};
    use state::fs;

    use super::{check_linux_setup_host, require_executable, validate_host};
    use crate::PlatformError;

    #[test]
    fn linux_setup_requires_systemd_and_kernel_5_15_or_newer() -> anyhow::Result<()> {
        let mut rows = Vec::new();
        for (name, init, kernel, accepted) in [
            ("foreign init", "tini", KernelVersion::new(5, 15, 0), false),
            (
                "old kernel",
                "systemd",
                KernelVersion::new(5, 14, 999),
                false,
            ),
            (
                "minimum kernel",
                "systemd",
                KernelVersion::new(5, 15, 0),
                true,
            ),
            ("newer kernel", "systemd", KernelVersion::new(6, 0, 0), true),
        ] {
            let result = validate_host(init, kernel);
            assert_eq!(result.is_ok(), accepted);
            let outcome = match result {
                Ok(()) => "accepted".to_owned(),
                Err(PlatformError::SystemIntegration(message)) => message,
                Err(error) => anyhow::bail!("unexpected host validation error: {error}"),
            };
            rows.push((name, outcome));
        }
        assert_debug_snapshot!(rows, @r#"
        [
            (
                "foreign init",
                "PV requires systemd as PID 1 on Linux; run `pv setup` on a systemd host",
            ),
            (
                "old kernel",
                "PV requires Linux kernel 5.15 or newer, found 5.14.999; upgrade the kernel before running `pv setup`",
            ),
            (
                "minimum kernel",
                "accepted",
            ),
            (
                "newer kernel",
                "accepted",
            ),
        ]
        "#);
        Ok(())
    }

    #[test]
    fn linux_setup_requires_a_regular_executable_and_follows_its_symlink() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let executable = directory.path().join("executable");
        fs::write_sensitive_file(&executable, "fixture, never executed")?;
        assert!(matches!(
            require_executable(executable.as_str()),
            Err(PlatformError::SystemIntegration(_))
        ));
        chmod(
            executable.as_std_path(),
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        )?;
        require_executable(executable.as_str())?;
        let link = directory.path().join("link");
        fs::symlink_file(&executable, &link)?;
        require_executable(link.as_str())?;
        for path in [
            directory.path().to_owned(),
            directory.path().join("missing"),
        ] {
            assert!(matches!(
                require_executable(path.as_str()),
                Err(PlatformError::SystemIntegration(_))
            ));
        }
        Ok(())
    }

    #[test]
    #[ignore = "run as dev (UID 1000) in a disposable Linux systemd test container"]
    fn linux_setup_reads_native_host_prerequisites() -> anyhow::Result<()> {
        check_linux_setup_host("unix:path=/run/user/1000/bus")?;
        Ok(())
    }
}
