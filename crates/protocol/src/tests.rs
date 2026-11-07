use crate::{
    DaemonEvent, DaemonResponse, ManagedResourceUpdateCheck, ManagedResourceUpdateCheckTrack,
    ManagedResourceUpdateStatus,
};

#[test]
fn protocol_messages_keep_their_wire_format() -> anyhow::Result<()> {
    let accepted = DaemonResponse::accepted("job accepted", "job-1");
    let update_check = DaemonResponse::ok_update_check(
        "Managed Resource update check completed",
        ManagedResourceUpdateCheck {
            managed_resources: vec![ManagedResourceUpdateCheckTrack {
                status: ManagedResourceUpdateStatus::UpdateAvailable,
                resource: "redis".to_string(),
                track: "8.8".to_string(),
                current_artifact_version: "8.8.0-pv1".to_string(),
                current_artifact_path: "/Users/me/.pv/resources/redis/8.8/releases/8.8.0-pv1"
                    .to_string(),
                latest_artifact_version: Some("8.8.1-pv1".to_string()),
                current_revocation: None,
                latest_revocation: None,
                blocked_by: None,
                reason: None,
            }],
        },
    );
    let download_progress = DaemonEvent::DownloadProgress {
        job_id: "job-1",
        resource: "redis",
        track: "8.8",
        artifact_version: "8.8.1-pv1",
        downloaded_bytes: 42,
        total_bytes: 100,
    };

    for response in [&accepted, &update_check] {
        let encoded = serde_json::to_value(response)?;
        let decoded = serde_json::from_value::<DaemonResponse>(encoded.clone())?;
        assert_eq!(serde_json::to_value(decoded)?, encoded);
    }
    let wire_format = serde_json::to_string_pretty(&[
        serde_json::to_value(&accepted)?,
        serde_json::to_value(&update_check)?,
        serde_json::to_value(&download_progress)?,
    ])?;
    insta::assert_snapshot!(wire_format);

    Ok(())
}
