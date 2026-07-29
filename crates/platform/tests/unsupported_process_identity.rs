#![cfg(not(target_os = "macos"))]

use platform::{PlatformCapability, PlatformError, PlatformTarget, inspect_process_identity};

#[test]
fn public_process_identity_inspection_rejects_unsupported_platform() -> anyhow::Result<()> {
    let target = PlatformTarget::current()?;
    let result = inspect_process_identity(1);

    assert!(matches!(
        result,
        Err(PlatformError::Unsupported {
            capability: PlatformCapability::ProcessInspection,
            target: error_target,
        }) if error_target == target
    ));

    Ok(())
}
