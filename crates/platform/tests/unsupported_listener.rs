#![cfg(not(target_os = "macos"))]

use platform::{
    PlatformCapability, PlatformError, PlatformTarget, loopback_tcp_listener_ports,
    loopback_tcp_port_has_listener,
};

#[test]
fn public_listener_inspection_rejects_unsupported_platform_before_inspection() -> anyhow::Result<()>
{
    let target = PlatformTarget::current()?;

    let ports_result = loopback_tcp_listener_ports();
    assert!(matches!(
        ports_result,
        Err(PlatformError::Unsupported {
            capability: PlatformCapability::ListenerInspection,
            target: error_target,
        }) if error_target == target
    ));

    let port_result = loopback_tcp_port_has_listener(45_000);
    assert!(matches!(
        port_result,
        Err(PlatformError::Unsupported {
            capability: PlatformCapability::ListenerInspection,
            target: error_target,
        }) if error_target == target
    ));

    Ok(())
}
