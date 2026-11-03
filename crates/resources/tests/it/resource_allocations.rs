use anyhow::{Result, anyhow};
use insta::assert_debug_snapshot;
use resources::registry;
use resources::{
    ConcreteTrackName, ResourceAllocationKind, ResourceAllocationName, ResourceName,
    ResourcesError, allocation_env_placeholders, generated_allocation_name,
    resource_env_placeholders,
};

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct AllocationNameSnapshot {
    resource_name: String,
    allocation_name: String,
    generated_name: String,
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "snapshot-only structure is read through derived Debug"
)]
struct PlaceholderContractSnapshot {
    resource: &'static str,
    allocation_kind: Option<String>,
    resource_placeholders: Vec<&'static str>,
    allocation_placeholders: Vec<&'static str>,
}

#[test]
fn resource_allocations_generate_resource_specific_names() -> Result<()> {
    let allocations = [
        ("mysql", "acme", "app-db"),
        ("postgres", "api", "analytics"),
        ("redis", "acme", "cache"),
        ("rustfs", "acme", "uploads_bucket"),
        ("mysql", "acme-store", "app-db"),
        ("redis", "acme-store", "cache_bucket"),
        ("rustfs", "acme-store", "cache_bucket"),
        ("pg", "acme", "app-db"),
        ("s3", "acme", "uploads_bucket"),
    ]
    .into_iter()
    .map(|(resource, project_slug, allocation)| {
        Ok(allocation_name_summary(generated_allocation_name(
            resource,
            project_slug,
            allocation,
        )?))
    })
    .collect::<Result<Vec<_>>>()?;

    assert_debug_snapshot!(allocations);

    Ok(())
}

#[test]
fn generated_allocation_names_enforce_sixty_three_character_limit() -> Result<()> {
    let sql_exact_allocation = "a".repeat(61);
    let redis_exact_allocation = "a".repeat(60);
    let rustfs_exact_allocation = "a".repeat(61);

    assert_eq!(
        generated_allocation_name("mysql", "a", &sql_exact_allocation)?
            .generated_name()
            .len(),
        63
    );
    assert_eq!(
        generated_allocation_name("redis", "a", &redis_exact_allocation)?
            .generated_name()
            .len(),
        63
    );
    assert_eq!(
        generated_allocation_name("rustfs", "a", &rustfs_exact_allocation)?
            .generated_name()
            .len(),
        63
    );

    let sql_too_long_allocation = "a".repeat(62);
    let redis_too_long_allocation = "a".repeat(61);
    let rustfs_too_long_allocation = "a".repeat(62);
    let errors = vec![
        allocation_error("mysql", "a", &sql_too_long_allocation)?,
        allocation_error("redis", "a", &redis_too_long_allocation)?,
        allocation_error("rustfs", "a", &rustfs_too_long_allocation)?,
    ];

    assert_debug_snapshot!(errors);

    Ok(())
}

#[test]
fn unsupported_resource_allocations_report_canonical_resource_names() -> Result<()> {
    let errors = ["mailpit", "mail", "php", "frankenphp", "composer"]
        .into_iter()
        .map(|resource| allocation_error(resource, "acme", "app"))
        .collect::<Result<Vec<_>>>()?;

    assert_debug_snapshot!(errors);

    Ok(())
}

#[test]
fn concrete_track_names_reject_latest_and_missing_tracks() -> Result<()> {
    let mysql = ResourceName::new("mysql")?;
    let concrete = ConcreteTrackName::required(&mysql, Some("8.0"))?;
    let track = concrete.clone().into_track_name();

    assert_eq!(concrete.as_str(), "8.0");
    assert_eq!(concrete.track_name().as_str(), "8.0");
    assert_eq!(track.as_str(), "8.0");

    let errors = vec![
        concrete_track_error(ConcreteTrackName::new("latest"))?,
        concrete_track_error(ConcreteTrackName::required(&mysql, None))?,
    ];

    assert_debug_snapshot!(errors);

    Ok(())
}

#[test]
fn placeholder_contracts_are_resource_specific_and_alias_aware() -> Result<()> {
    let contracts = [
        "php",
        "frankenphp",
        "composer",
        "mysql",
        "postgres",
        "redis",
        "mailpit",
        "rustfs",
    ]
    .into_iter()
    .map(placeholder_contract_summary)
    .collect::<Result<Vec<_>>>()?;

    assert_eq!(
        registry::resolve("mysql")?.allocation_kind(),
        Some(ResourceAllocationKind::SqlDatabase)
    );
    assert_eq!(
        registry::resolve("redis")?.allocation_kind(),
        Some(ResourceAllocationKind::RedisPrefix)
    );
    assert_eq!(
        registry::resolve("rustfs")?.allocation_kind(),
        Some(ResourceAllocationKind::RustfsBucket)
    );
    assert_eq!(
        resource_env_placeholders("pg")?,
        resource_env_placeholders("postgres")?
    );
    assert_eq!(
        allocation_env_placeholders("s3")?,
        allocation_env_placeholders("rustfs")?
    );
    assert!(!resource_env_placeholders("mysql")?.contains(&"smtp_host"));
    assert!(!allocation_env_placeholders("redis")?.contains(&"bucket"));
    assert!(allocation_env_placeholders("mysql")?.contains(&"database"));
    assert!(allocation_env_placeholders("redis")?.contains(&"prefix"));
    assert!(allocation_env_placeholders("rustfs")?.contains(&"bucket"));

    assert_debug_snapshot!(contracts);

    Ok(())
}

fn allocation_name_summary(name: ResourceAllocationName) -> AllocationNameSnapshot {
    AllocationNameSnapshot {
        resource_name: name.resource_name().as_str().to_string(),
        allocation_name: name.allocation_name().to_string(),
        generated_name: name.generated_name().to_string(),
    }
}

fn allocation_error(
    resource: &str,
    project_slug: &str,
    allocation: &str,
) -> Result<ResourcesError> {
    match generated_allocation_name(resource, project_slug, allocation) {
        Ok(name) => Err(anyhow!("expected allocation error, got {name:?}")),
        Err(error) => Ok(error),
    }
}

fn concrete_track_error(
    result: std::result::Result<ConcreteTrackName, ResourcesError>,
) -> Result<ResourcesError> {
    match result {
        Ok(track) => Err(anyhow!("expected concrete track error, got {track:?}")),
        Err(error) => Ok(error),
    }
}

fn placeholder_contract_summary(resource: &'static str) -> Result<PlaceholderContractSnapshot> {
    let descriptor = registry::resolve(resource)?;

    Ok(PlaceholderContractSnapshot {
        resource: descriptor.name(),
        allocation_kind: descriptor
            .allocation_kind()
            .map(|allocation_kind| format!("{allocation_kind:?}")),
        resource_placeholders: resource_env_placeholders(resource)?.to_vec(),
        allocation_placeholders: allocation_env_placeholders(resource)?.to_vec(),
    })
}
