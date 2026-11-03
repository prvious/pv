#![cfg(target_os = "linux")]

use platform::{
    PlatformError, PrivilegedHelperLifecycleAction, run_privileged_helper_lifecycle,
    serve_privileged_helper,
};
use rustix::process::getuid;

#[test]
fn helper_refuses_non_root_startup_and_lifecycle_before_reading_installation_metadata() {
    assert!(
        !getuid().is_root(),
        "run the ordinary integration suite as an unprivileged user"
    );
    assert!(matches!(
        serve_privileged_helper(),
        Err(PlatformError::PrivilegedHelperAuthentication(_))
    ));
    for action in [
        PrivilegedHelperLifecycleAction::Install,
        PrivilegedHelperLifecycleAction::Replace,
        PrivilegedHelperLifecycleAction::Remove,
    ] {
        assert!(matches!(
            run_privileged_helper_lifecycle(action, getuid().as_raw(), "invalid", "invalid", 0),
            Err(PlatformError::PrivilegedHelperAuthentication(_))
        ));
    }
}
