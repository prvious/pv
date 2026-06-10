use std::collections::BTreeMap;

use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::create_bucket::CreateBucketError;
use aws_sdk_s3::{Client, Config};
use object_store::ObjectStoreExt;
use object_store::aws::{AmazonS3Builder, AmazonS3ConfigKey};
use object_store::client::ClientConfigKey;
use object_store::path::Path;
use state::{
    Database, EnvContextValues, PvPaths, ResourceAllocationRecord, ResourceAllocationStatus,
};

use crate::managed_resources::{
    ManagedResourcePortSpec, ManagedResourceReadiness, ManagedResourceRuntimeAdapter,
    ManagedResourceRuntimeContext, RESOURCE_HOST,
};
use crate::{DaemonError, ProcessSpec, ReadinessCheck};

const RESOURCE_NAME: &str = "rustfs";
const ACCESS_KEY: &str = "pv-rustfs";
const REGION: &str = "us-east-1";
const PROBE_OBJECT: &str = "__pv_rustfs_probe";
const PROBE_CONTENT: &str = "pv rustfs probe";
const PORTS: &[ManagedResourcePortSpec] = &[
    ManagedResourcePortSpec {
        name: "api",
        preferred_port: 9000,
    },
    ManagedResourcePortSpec {
        name: "console",
        preferred_port: 9001,
    },
];

#[derive(Clone, Debug)]
pub(crate) struct RustfsRuntimeAdapter;

impl ManagedResourceRuntimeAdapter for RustfsRuntimeAdapter {
    fn resource_name(&self) -> &'static str {
        RESOURCE_NAME
    }

    fn artifact_adapter(&self) -> Result<resources::RuntimeArtifactAdapter, DaemonError> {
        resources::rustfs_adapter().map_err(Into::into)
    }

    fn port_specs(&self) -> &'static [ManagedResourcePortSpec] {
        PORTS
    }

    fn build_process_spec(
        &self,
        paths: &PvPaths,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ProcessSpec, DaemonError> {
        let api_port = required_port(context, "api")?;
        let console_port = required_port(context, "console")?;
        let config_path = paths.resource_runtime_config(&context.resource_name, &context.track);
        let config = serde_json::json!({
            "resource": context.resource_name,
            "track": context.track,
            "api_port": api_port,
            "console_port": console_port,
            "data_dir": context.data_dir.as_str(),
        });

        state::fs::write_sensitive_file(&config_path, &serde_json::to_string_pretty(&config)?)?;

        Ok(ProcessSpec {
            name: format!("{}-{}", context.resource_name, context.track),
            command: self
                .artifact_adapter()?
                .executable_path(&context.artifact_path),
            arguments: vec![
                "--address".to_string(),
                format!("{RESOURCE_HOST}:{api_port}"),
                "--console-address".to_string(),
                format!("{RESOURCE_HOST}:{console_port}"),
                context.data_dir.to_string(),
            ],
            private_environment: process_environment(&context.env)?,
            config_path,
            log_path: paths.resource_log(&context.resource_name, &context.track),
            pid_path: paths.resource_pid(&context.resource_name, &context.track),
            metadata_path: paths.resource_runtime_metadata(&context.resource_name, &context.track),
            resource_name: context.resource_name.clone(),
            track: context.track.clone(),
        })
    }

    fn readiness(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<ManagedResourceReadiness, DaemonError> {
        Ok(ReadinessCheck::Http {
            host: RESOURCE_HOST.to_string(),
            port: required_port(context, "api")?,
            path: "/".to_string(),
        }
        .into())
    }

    fn resource_env(
        &self,
        context: &ManagedResourceRuntimeContext,
    ) -> Result<EnvContextValues, DaemonError> {
        rustfs_resource_env(context)
    }

    fn reconcile_allocations<'a>(
        &'a self,
        _paths: &'a PvPaths,
        database: &'a mut Database,
        context: &'a ManagedResourceRuntimeContext,
        resource_env: &'a EnvContextValues,
        allocations: &'a [ResourceAllocationRecord],
    ) -> super::ManagedResourceAllocationFuture<'a> {
        Box::pin(async move {
            let client = s3_client(resource_env)?;

            for allocation in allocations {
                let bucket = allocation.generated_name.as_str();
                create_bucket(&client, bucket).await?;
                verify_object_operations(resource_env, bucket).await?;
                if allocation.status == ResourceAllocationStatus::Ready {
                    continue;
                }

                let allocation_env = allocation_env(bucket);
                database.mark_resource_allocation_ready(
                    &allocation.project_id,
                    &context.resource_name,
                    &context.track,
                    &allocation.allocation_name,
                    &allocation_env,
                )?;
            }

            Ok(())
        })
    }
}

fn rustfs_resource_env(
    context: &ManagedResourceRuntimeContext,
) -> Result<EnvContextValues, DaemonError> {
    let api_port = required_port(context, "api")?;
    let endpoint = format!("http://{RESOURCE_HOST}:{api_port}");
    let (access_key, secret_key) = if context.env.is_empty() {
        (ACCESS_KEY.to_string(), generated_secret_key()?)
    } else {
        (
            required_env_value(&context.env, "access_key")?,
            required_env_value(&context.env, "secret_key")?,
        )
    };

    Ok(BTreeMap::from([
        ("access_key".to_string(), access_key),
        ("secret_key".to_string(), secret_key),
        ("endpoint".to_string(), endpoint.clone()),
        ("host".to_string(), RESOURCE_HOST.to_string()),
        ("port".to_string(), api_port.to_string()),
        ("url".to_string(), endpoint),
    ]))
}

fn allocation_env(bucket: &str) -> EnvContextValues {
    BTreeMap::from([("bucket".to_string(), bucket.to_string())])
}

fn process_environment(
    resource_env: &EnvContextValues,
) -> Result<BTreeMap<String, String>, DaemonError> {
    Ok(BTreeMap::from([
        (
            "RUSTFS_ACCESS_KEY".to_string(),
            required_env_value(resource_env, "access_key")?,
        ),
        (
            "RUSTFS_SECRET_KEY".to_string(),
            required_env_value(resource_env, "secret_key")?,
        ),
    ]))
}

async fn create_bucket(client: &Client, bucket: &str) -> Result<(), DaemonError> {
    match client.create_bucket().bucket(bucket).send().await {
        Ok(_output) => Ok(()),
        Err(error) if bucket_already_exists(&error) => Ok(()),
        Err(error) => Err(rustfs_admin_error(
            format!("failed to create bucket `{bucket}`"),
            error,
        )),
    }
}

async fn verify_object_operations(
    resource_env: &EnvContextValues,
    bucket: &str,
) -> Result<(), DaemonError> {
    let store = AmazonS3Builder::new()
        .with_config(AmazonS3ConfigKey::Region, REGION)
        .with_config(AmazonS3ConfigKey::Bucket, bucket)
        .with_config(
            AmazonS3ConfigKey::AccessKeyId,
            required_env_value(resource_env, "access_key")?,
        )
        .with_config(
            AmazonS3ConfigKey::SecretAccessKey,
            required_env_value(resource_env, "secret_key")?,
        )
        .with_config(
            AmazonS3ConfigKey::Endpoint,
            required_env_value(resource_env, "endpoint")?,
        )
        .with_config(AmazonS3ConfigKey::VirtualHostedStyleRequest, "false")
        .with_config(
            AmazonS3ConfigKey::Client(ClientConfigKey::AllowHttp),
            "true",
        )
        .build()
        .map_err(|source| rustfs_admin_error("failed to configure object store", source))?;
    let path = Path::from(PROBE_OBJECT);

    store
        .put(&path, PROBE_CONTENT.to_string().into())
        .await
        .map_err(|source| {
            rustfs_admin_error(
                format!("failed to write probe object in `{bucket}`"),
                source,
            )
        })?;
    store.head(&path).await.map_err(|source| {
        rustfs_admin_error(
            format!("failed to read probe object metadata in `{bucket}`"),
            source,
        )
    })?;

    Ok(())
}

fn s3_client(resource_env: &EnvContextValues) -> Result<Client, DaemonError> {
    let config = Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .credentials_provider(Credentials::new(
            required_env_value(resource_env, "access_key")?,
            required_env_value(resource_env, "secret_key")?,
            None,
            None,
            "pv-rustfs",
        ))
        .region(Region::new(REGION))
        .endpoint_url(required_env_value(resource_env, "endpoint")?)
        .force_path_style(true)
        .build();

    Ok(Client::from_conf(config))
}

fn bucket_already_exists(error: &SdkError<CreateBucketError>) -> bool {
    error.as_service_error().is_some_and(|error| {
        error.is_bucket_already_exists() || error.is_bucket_already_owned_by_you()
    })
}

fn required_port(
    context: &ManagedResourceRuntimeContext,
    port_name: &str,
) -> Result<u16, DaemonError> {
    context
        .ports
        .get(port_name)
        .copied()
        .ok_or_else(|| DaemonError::ManagedResourcePortMissing {
            resource: context.resource_name.clone(),
            track: context.track.clone(),
            port: port_name.to_string(),
        })
}

fn required_env_value(env: &EnvContextValues, key: &str) -> Result<String, DaemonError> {
    env.get(key)
        .cloned()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DaemonError::UnexpectedProtocolResponse {
            reason: format!("RustFS resource env is missing `{key}`"),
        })
}

fn generated_secret_key() -> Result<String, DaemonError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|source| rustfs_admin_error("failed to generate RustFS secret key", source))?;

    Ok(hex_lower(bytes))
}

fn hex_lower(bytes: impl IntoIterator<Item = u8>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(32);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }

    encoded
}

fn rustfs_admin_error(context: impl Into<String>, source: impl std::fmt::Display) -> DaemonError {
    DaemonError::UnexpectedProtocolResponse {
        reason: format!("RustFS admin error: {}; {source}", context.into()),
    }
}
