use anyhow::{Result, ensure};
use camino::Utf8Path;
use platform::{
    PrivilegedHelperClient, UNPRIVILEGED_PORTS_PATH, UnprivilegedPortsState,
    inspect_unprivileged_ports,
};
use state::fs;

use super::{HelperFixture, checksum, root, root_text, with_helper_fixture};

const LIVE_VALUE: &str = "/proc/sys/net/ipv4/ip_unprivileged_port_start";
const OTHER_FILE: &str = "/etc/sysctl.d/50-pv-system-test-ports.conf";

#[test]
#[ignore = "requires PV_LINUX_SYSTEM_TESTS=1, the built helper and a disposable systemd machine"]
fn kernel_port_setting_applies_and_removes_only_pvs_own_file() -> Result<()> {
    with_helper_fixture(exercise)
}

fn exercise(fixture: &mut HelperFixture, helper: &Utf8Path) -> Result<()> {
    for path in [UNPRIVILEGED_PORTS_PATH, OTHER_FILE] {
        ensure!(
            !fs::path_entry_exists(Utf8Path::new(path))?,
            "refusing existing {path}"
        );
    }
    ensure!(
        live_value()? == "1024",
        "the disposable machine must start with the kernel's default"
    );
    assert_eq!(
        inspect_unprivileged_ports(),
        UnprivilegedPortsState::Missing
    );
    fixture.install(helper, &checksum(helper)?)?;

    let result = apply_and_remove(fixture);
    let cleanup = root(&[
        "/usr/bin/rm",
        "-f",
        "--",
        UNPRIVILEGED_PORTS_PATH,
        OTHER_FILE,
    ])
    .and_then(|_output| {
        root(&[
            "/usr/sbin/sysctl",
            "-q",
            "-w",
            "net.ipv4.ip_unprivileged_port_start=1024",
        ])
    });
    result.and(cleanup.map(|_output| ()))
}

fn apply_and_remove(fixture: &HelperFixture) -> Result<()> {
    PrivilegedHelperClient.apply_unprivileged_ports()?;
    assert_eq!(
        fs::read_to_string(Utf8Path::new(UNPRIVILEGED_PORTS_PATH))?,
        "# Managed by PV\nnet.ipv4.ip_unprivileged_port_start=80\n"
    );
    assert_eq!(
        root_text(&["/usr/bin/stat", "-c", "%u:%g:%a", UNPRIVILEGED_PORTS_PATH])?.trim(),
        "0:0:644"
    );
    assert_eq!(live_value()?, "80");
    assert_eq!(
        inspect_unprivileged_ports(),
        UnprivilegedPortsState::Current
    );

    PrivilegedHelperClient.remove_unprivileged_ports()?;
    assert!(!fs::path_entry_exists(Utf8Path::new(
        UNPRIVILEGED_PORTS_PATH
    ))?);
    assert_eq!(live_value()?, "1024");

    // Another tool that relies on the setting keeps it after PV's removal.
    fixture.root_file(
        "other-ports.conf",
        "net.ipv4.ip_unprivileged_port_start = 80\n",
        OTHER_FILE,
        "0644",
    )?;
    PrivilegedHelperClient.apply_unprivileged_ports()?;
    PrivilegedHelperClient.remove_unprivileged_ports()?;
    assert!(!fs::path_entry_exists(Utf8Path::new(
        UNPRIVILEGED_PORTS_PATH
    ))?);
    assert!(fs::path_entry_exists(Utf8Path::new(OTHER_FILE))?);
    assert_eq!(live_value()?, "80");

    // Without PV's file, an administrator's live value is theirs.
    root(&["/usr/bin/rm", "-f", "--", OTHER_FILE])?;
    PrivilegedHelperClient.remove_unprivileged_ports()?;
    assert_eq!(live_value()?, "80");

    // A lower value another tool chose survives PV's setup and removal, and PV's file keeps it.
    fixture.root_file(
        "other-ports.conf",
        "net.ipv4.ip_unprivileged_port_start = 0\n",
        OTHER_FILE,
        "0644",
    )?;
    root(&[
        "/usr/sbin/sysctl",
        "-q",
        "-w",
        "net.ipv4.ip_unprivileged_port_start=0",
    ])?;
    PrivilegedHelperClient.apply_unprivileged_ports()?;
    assert_eq!(
        fs::read_to_string(Utf8Path::new(UNPRIVILEGED_PORTS_PATH))?,
        "# Managed by PV\nnet.ipv4.ip_unprivileged_port_start=0\n"
    );
    assert_eq!(live_value()?, "0");
    PrivilegedHelperClient.remove_unprivileged_ports()?;
    assert!(!fs::path_entry_exists(Utf8Path::new(
        UNPRIVILEGED_PORTS_PATH
    ))?);
    assert_eq!(live_value()?, "0");

    Ok(())
}

fn live_value() -> Result<String> {
    Ok(fs::read_to_string(Utf8Path::new(LIVE_VALUE))?
        .trim()
        .to_owned())
}
