use platform::BootSessionId;

#[test]
fn boot_session_identity_validates_and_normalizes_recorded_evidence() -> anyhow::Result<()> {
    let lowercase = "12345678-9abc-def0-1234-56789abcdef0";
    let identity: BootSessionId = serde_json::from_str(&format!("\"{lowercase}\""))?;
    assert_eq!(
        serde_json::to_string(&identity)?,
        "\"12345678-9ABC-DEF0-1234-56789ABCDEF0\"",
    );
    for invalid in [
        "",
        "invalid",
        "00000000-0000-0000-0000-000000000000",
        "123456789abcdef0123456789abcdef0",
        "12345678_9abc-def0-1234-56789abcdef0",
        "12345678-9abc-def0-1234-56789abcdefg",
    ] {
        assert!(serde_json::from_str::<BootSessionId>(&format!("\"{invalid}\"")).is_err());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn native_boot_session_identity_is_stable_across_reads() -> anyhow::Result<()> {
    assert_eq!(
        platform::current_boot_session_id()?,
        platform::current_boot_session_id()?
    );
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[test]
fn unsupported_boot_session_identity_fails_without_inventing_evidence() -> anyhow::Result<()> {
    assert!(matches!(
        platform::current_boot_session_id(),
        Err(platform::PlatformError::Unsupported {
            capability: platform::PlatformCapability::ProcessInspection,
            target,
        }) if target == platform::PlatformTarget::current()?
    ));
    Ok(())
}
