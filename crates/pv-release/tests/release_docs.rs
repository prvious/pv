use anyhow::Result;
use camino::Utf8Path;
use pv_release::defaults::ManifestDefaults;
use resources::ResourceName;

#[test]
fn release_docs_cover_rc_sections_and_default_track_matrix() -> Result<()> {
    let Some(workspace_root) = Utf8Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Utf8Path::parent)
    else {
        anyhow::bail!("pv-release crate is not under the workspace crates directory");
    };
    let checklist = read_to_string(&workspace_root.join("docs/release/rc-checklist.md"))?;
    let user_docs = read_to_string(&workspace_root.join("docs/user/README.md"))?;
    let implementation = read_to_string(&workspace_root.join("IMPLEMENTATION.md"))?;
    let artifact_docs = read_to_string(&workspace_root.join("release/artifacts/README.md"))?;
    let default_tracks =
        ManifestDefaults::load(&workspace_root.join("release/artifacts/default-tracks.toml"))?;

    for heading in [
        "## Artifact Publication",
        "## Fresh Install",
        "## Setup And System",
        "## Project Flow",
        "## Managed Resources",
        "## Update",
        "## Diagnostics",
        "## Uninstall",
        "## Blockers",
        "## Sign-Off",
    ] {
        assert!(
            checklist.contains(heading),
            "RC checklist is missing required PV-125 section `{heading}`"
        );
    }

    for evidence_field in [
        "RC version",
        "Commit",
        "Tester",
        "macOS version",
        "Architecture",
        "Target scope",
        "Installer URL",
        "App manifest URL",
        "Artifact manifest URL",
        "Workflow run IDs",
    ] {
        assert!(
            checklist.contains(evidence_field),
            "RC checklist is missing evidence field `{evidence_field}`"
        );
    }

    for (resource, expected_track) in [
        ("php", "8.5"),
        ("frankenphp", "8.5"),
        ("composer", "2"),
        ("mysql", "8.4"),
        ("postgres", "18"),
        ("redis", "8.8"),
        ("mailpit", "1"),
        ("rustfs", "1"),
    ] {
        assert_eq!(
            default_track(&default_tracks, resource)?,
            expected_track,
            "committed default track drifted for `{resource}`"
        );
    }

    let combined_docs = format!("{checklist}\n{user_docs}\n{implementation}\n{artifact_docs}");
    for required_text in [
        "PHP/FrankenPHP `8.5`",
        "Composer `2`",
        "MySQL `8.4`",
        "Postgres `18`",
        "Redis `8.8`",
        "Mailpit `1`",
        "RustFS `1`",
        "Apple Silicon/staging RC",
        "conditional `darwin-amd64` gate",
    ] {
        assert!(
            combined_docs.contains(required_text),
            "release docs are missing default-track or platform text `{required_text}`"
        );
    }

    Ok(())
}

fn default_track(defaults: &ManifestDefaults, resource: &str) -> Result<String> {
    let resource_name = ResourceName::new(resource)?;
    Ok(defaults
        .default_track_for(&resource_name)
        .map(ToString::to_string)
        .unwrap_or_default())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release docs sync test reads committed documentation fixtures directly"
)]
fn read_to_string(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}
