#[cfg(target_env = "gnu")]
use std::ffi::CStr;

use procfs::KernelVersion;
use procfs::process::Process;
use rustix::fs::{Access, FileType, access, stat};

use crate::PlatformError;
use crate::daemon_registration::check_user_manager;
use crate::helper::SUDO_EXECUTABLE;

mod resolved;

pub(crate) const CERTUTIL_EXECUTABLE: &str = "/usr/bin/certutil";

/// Verifies a fresh `.test` lookup through the configured stub after reconciliation.
pub fn check_linux_setup_dns() -> Result<(), PlatformError> {
    resolved::check_test().map_err(|error| {
        PlatformError::SystemIntegration(format!(
            ".test DNS verification failed after reconciliation: {error}; check `pv status` and `pv dns:status`, then repair with `pv dns:install` before retrying `pv setup`"
        ))
    })
}

/// Checks the native host and required executables before Linux setup.
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
    validate_glibc_version(runtime_glibc_version()?)?;
    require_executable(SUDO_EXECUTABLE).map_err(|error| {
        PlatformError::SystemIntegration(format!(
            "{error}; install the sudo package before running `pv setup`"
        ))
    })?;
    require_executable(CERTUTIL_EXECUTABLE).map_err(|error| {
        PlatformError::SystemIntegration(format!(
            "{error}; install libnss3-tools (Debian/Ubuntu) or nss-tools (Fedora) before running `pv setup`"
        ))
    })?;
    check_user_manager(session_address).map_err(|error| {
        PlatformError::SystemIntegration(format!(
            "{error}; run `pv setup` from a logged-in systemd user session"
        ))
    })?;
    resolved::check_stub()
}

fn runtime_glibc_version() -> Result<&'static str, PlatformError> {
    #[cfg(target_env = "gnu")]
    {
        // SAFETY: GNU libc returns a non-null pointer to its immutable, static,
        // NUL-terminated version array, which remains valid for this process.
        unsafe { CStr::from_ptr(libc::gnu_get_libc_version()) }
            .to_str()
            .map_err(|error| {
                PlatformError::SystemIntegration(format!(
                    "could not read the runtime glibc version: {error}; use GNU glibc 2.35 or newer"
                ))
            })
    }
    #[cfg(not(target_env = "gnu"))]
    {
        Err(PlatformError::SystemIntegration(
            "PV requires GNU glibc 2.35 or newer on Linux; use a glibc-based distribution"
                .to_owned(),
        ))
    }
}

fn validate_glibc_version(version: &str) -> Result<(), PlatformError> {
    let mut components = version.split('.').map(|component| {
        component
            .parse::<u32>()
            .ok()
            .filter(|_| component.bytes().all(|byte| byte.is_ascii_digit()))
    });
    if matches!(
        (components.next(), components.next()),
        (Some(Some(major)), Some(Some(minor))) if (major, minor) >= (2, 35)
    ) && components.all(|component| component.is_some())
    {
        return Ok(());
    }
    Err(PlatformError::SystemIntegration(format!(
        "PV requires glibc 2.35 or newer, found {version:?}; upgrade glibc before running `pv setup`"
    )))
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

    use super::{
        check_linux_setup_dns, check_linux_setup_host, require_executable, validate_glibc_version,
        validate_host,
    };
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
    fn linux_setup_requires_glibc_2_35_or_newer() -> anyhow::Result<()> {
        let mut rows = Vec::new();
        for (version, accepted) in [
            ("2.34", false),
            ("2.9", false),
            ("2.35", true),
            ("2.35.1", true),
            ("2.100", true),
            ("3.0", true),
            ("+2.35", false),
            ("2.35.junk", false),
            ("2.35.", false),
            ("2.35.4294967296", false),
            ("2", false),
            ("", false),
        ] {
            let result = validate_glibc_version(version);
            assert_eq!(result.is_ok(), accepted, "{version:?}");
            let outcome = match result {
                Ok(()) => "accepted".to_owned(),
                Err(PlatformError::SystemIntegration(message)) => message,
                Err(error) => anyhow::bail!("unexpected glibc validation error: {error}"),
            };
            rows.push((version, outcome));
        }
        assert_debug_snapshot!(rows, @r#"
        [
            (
                "2.34",
                "PV requires glibc 2.35 or newer, found \"2.34\"; upgrade glibc before running `pv setup`",
            ),
            (
                "2.9",
                "PV requires glibc 2.35 or newer, found \"2.9\"; upgrade glibc before running `pv setup`",
            ),
            (
                "2.35",
                "accepted",
            ),
            (
                "2.35.1",
                "accepted",
            ),
            (
                "2.100",
                "accepted",
            ),
            (
                "3.0",
                "accepted",
            ),
            (
                "+2.35",
                "PV requires glibc 2.35 or newer, found \"+2.35\"; upgrade glibc before running `pv setup`",
            ),
            (
                "2.35.junk",
                "PV requires glibc 2.35 or newer, found \"2.35.junk\"; upgrade glibc before running `pv setup`",
            ),
            (
                "2.35.",
                "PV requires glibc 2.35 or newer, found \"2.35.\"; upgrade glibc before running `pv setup`",
            ),
            (
                "2.35.4294967296",
                "PV requires glibc 2.35 or newer, found \"2.35.4294967296\"; upgrade glibc before running `pv setup`",
            ),
            (
                "2",
                "PV requires glibc 2.35 or newer, found \"2\"; upgrade glibc before running `pv setup`",
            ),
            (
                "",
                "PV requires glibc 2.35 or newer, found \"\"; upgrade glibc before running `pv setup`",
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

    #[test]
    #[ignore = "run as dev (UID 1000) with the configured stub in a disposable Linux systemd test container"]
    fn linux_setup_verifies_native_test_dns() -> anyhow::Result<()> {
        check_linux_setup_dns()?;
        Ok(())
    }
}
