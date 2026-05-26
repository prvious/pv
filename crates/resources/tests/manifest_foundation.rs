use anyhow::Result;
use insta::assert_debug_snapshot;
use resources::registry;
use resources::{ArtifactPlatform, TargetPlatform};
use resources::{ArtifactVersion, ResourceName, Sha256Digest, TrackName};

#[test]
fn registry_lists_all_pv_managed_artifact_resources() -> Result<()> {
    let names = registry::all()
        .iter()
        .map(|descriptor| descriptor.name())
        .collect::<Vec<_>>();

    assert_debug_snapshot!(names);

    Ok(())
}

#[test]
fn registry_normalizes_compiled_in_aliases() -> Result<()> {
    assert_eq!(registry::resolve("postgresql")?.name(), "postgres");
    assert_eq!(registry::resolve("pg")?.name(), "postgres");
    assert_eq!(registry::resolve("mail")?.name(), "mailpit");
    assert_eq!(registry::resolve("s3")?.name(), "rustfs");
    assert!(registry::resolve("postgresql")?.is_alias("postgresql"));
    assert!(registry::resolve("mysql")?.is_canonical("mysql"));

    Ok(())
}

#[test]
fn identity_types_reject_empty_values_and_bad_checksums() -> Result<()> {
    assert!(ResourceName::new("").is_err());
    assert!(TrackName::new("").is_err());
    assert!(ArtifactVersion::new("").is_err());
    assert!(Sha256Digest::new("not-a-sha").is_err());
    assert!(Sha256Digest::new(&"a".repeat(64)).is_ok());

    Ok(())
}

#[test]
fn platform_matching_prefers_exact_matches_over_any() -> Result<()> {
    let target = TargetPlatform::new("darwin-arm64")?;

    assert!(ArtifactPlatform::new("darwin-arm64")?.matches(target));
    assert!(ArtifactPlatform::new("any")?.matches(target));
    assert!(!ArtifactPlatform::new("darwin-amd64")?.matches(target));

    Ok(())
}
