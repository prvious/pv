use futures_util::StreamExt;
use serde_json::json;
use tokio::io::duplex;

use crate::{
    DaemonCommand, DaemonRequest, DaemonResponse, ManagedResourceUpdateCheck,
    ManagedResourceUpdateCheckTrack, ManagedResourceUpdateStatus, PROTOCOL_VERSION, ResponseStatus,
    transport, write_line,
};

#[test]
fn managed_resource_update_check_command_bumps_protocol_version() -> anyhow::Result<()> {
    let request = DaemonRequest {
        protocol_version: PROTOCOL_VERSION,
        command: DaemonCommand::ManagedResourceUpdateCheck,
    };

    assert_eq!(PROTOCOL_VERSION, 2);
    assert_eq!(
        serde_json::to_value(&request)?,
        json!({
            "protocol_version": 2,
            "command": "managed_resource_update_check",
        })
    );

    Ok(())
}

#[test]
fn response_envelope_round_trips_through_protocol_type() -> anyhow::Result<()> {
    let response = DaemonResponse::accepted("job accepted", "job-1");
    let encoded = serde_json::to_value(&response)?;

    assert_eq!(
        encoded,
        json!({
            "type": "response",
            "protocol_version": PROTOCOL_VERSION,
            "status": "accepted",
            "message": "job accepted",
            "job_id": "job-1",
        })
    );

    let decoded = serde_json::from_value::<DaemonResponse>(encoded)?;

    assert_eq!(decoded.status(), ResponseStatus::Accepted);
    assert_eq!(decoded.message(), "job accepted");
    assert_eq!(decoded.job_id(), Some("job-1"));

    Ok(())
}

#[test]
fn update_check_response_round_trips_with_managed_resources() -> anyhow::Result<()> {
    let response = DaemonResponse::ok_update_check(
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
    let encoded = serde_json::to_value(&response)?;

    assert_eq!(
        encoded,
        json!({
            "type": "response",
            "protocol_version": PROTOCOL_VERSION,
            "status": "ok",
            "message": "Managed Resource update check completed",
            "update_check": {
                "managed_resources": [
                    {
                        "status": "update_available",
                        "resource": "redis",
                        "track": "8.8",
                        "current_artifact_version": "8.8.0-pv1",
                        "current_artifact_path": "/Users/me/.pv/resources/redis/8.8/releases/8.8.0-pv1",
                        "latest_artifact_version": "8.8.1-pv1",
                        "current_revocation": null,
                        "latest_revocation": null,
                        "blocked_by": null,
                        "reason": null
                    }
                ]
            }
        })
    );

    let decoded = serde_json::from_value::<DaemonResponse>(encoded)?;

    assert_eq!(decoded.status(), ResponseStatus::Ok);
    assert_eq!(
        decoded
            .update_check()
            .map(|check| check.managed_resources.len()),
        Some(1)
    );

    Ok(())
}

#[tokio::test]
async fn transport_frames_generic_async_streams() -> anyhow::Result<()> {
    let (client, server) = duplex(1024);
    let mut writer = transport(client);
    let mut reader = transport(server);

    write_line(&mut writer, &DaemonResponse::ok("daemon healthy")).await?;

    let Some(line) = reader.next().await else {
        anyhow::bail!("reader closed before receiving a protocol line");
    };

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line?)?,
        json!({
            "type": "response",
            "protocol_version": PROTOCOL_VERSION,
            "status": "ok",
            "message": "daemon healthy",
        })
    );

    Ok(())
}
