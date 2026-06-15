use anyhow::Result;
use camino::Utf8Path;
use insta::assert_snapshot;

const ARTIFACT_RECIPES_UPLOAD_PATHS: [&str; 3] = [
    "${{ runner.temp }}/pv-artifacts/*.tar.gz",
    "${{ runner.temp }}/pv-artifacts/manifest.json",
    "${{ runner.temp }}/pv-records/**/*.json",
];
const APP_RELEASE_WORKFLOW_PATH: &str = ".github/workflows/app-release.yml";
const APP_PUBLICATION_WORKFLOW_PATH: &str = ".github/workflows/app-publication.yml";

#[test]
fn artifact_recipes_defaults_defer_staticphp_unstable_lanes() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = read_file(&workspace_root.join(".github/workflows/artifact-recipes.yml"))?;
    let summary = format!(
        "track_default={}\nplatform_default={}\nplatform_description={}\nplatform_matrices={:?}\nstaticphp_comment_present={}\nstaticphp_work_cleanup_restores_write_permission={}",
        input_default(&workflow, "track").unwrap_or(""),
        input_default(&workflow, "platform").unwrap_or(""),
        input_description(&workflow, "platform").unwrap_or(""),
        platform_matrices(&workflow),
        workflow.contains("StaticPHP v3"),
        workflow.contains("chmod -R u+w \"$PV_ARTIFACT_OUT_DIR/work\""),
    );

    assert_snapshot!(summary, @r#"
    track_default=all
    platform_default=all
    platform_description=Artifact platform: all uses the current preview matrix (currently darwin-arm64); choose darwin-arm64 or darwin-amd64 explicitly for one platform
    platform_matrices=["platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}", "platform: [any]", "platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}", "platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}", "platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}", "platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}", "platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}"]
    staticphp_comment_present=true
    staticphp_work_cleanup_restores_write_permission=true
    "#);

    Ok(())
}

#[test]
fn release_artifacts_readme_documents_current_recipe_validation() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let readme = read_file(&workspace_root.join("release/artifacts/README.md"))?;
    let summary = format!(
        "uses_current_recipe_count_wording={}\nshellchecks_all_recipe_scripts={}",
        !readme.contains("Both recipe TOML files"),
        readme.contains(
            "shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/*/*.sh"
        ),
    );

    assert_snapshot!(summary, @"
    uses_current_recipe_count_wording=true
    shellchecks_all_recipe_scripts=true
    ");

    Ok(())
}

#[test]
fn artifact_recipes_builds_resource_lanes_in_parallel() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = read_file(&workspace_root.join(".github/workflows/artifact-recipes.yml"))?;
    let summary = format!(
        "jobs={:?}\nupload_steps={}\narchive_upload_paths={}\nmanifest_upload_paths={}\nrecord_upload_paths={}\nrecipe_track_envs={}\ntrack_upload_names={}\nstaticphp_failure_logs={}",
        workflow_job_ids(&workflow),
        workflow.matches("uses: actions/upload-artifact@v7").count(),
        workflow.matches(ARTIFACT_RECIPES_UPLOAD_PATHS[0]).count(),
        workflow.matches(ARTIFACT_RECIPES_UPLOAD_PATHS[1]).count(),
        workflow.matches(ARTIFACT_RECIPES_UPLOAD_PATHS[2]).count(),
        workflow
            .matches("PV_RECIPE_TRACK: ${{ matrix.track }}")
            .count(),
        workflow
            .matches("${{ matrix.track }}-${{ matrix.platform }}-${{ github.run_id }}")
            .count(),
        workflow.contains(
            "pv-artifact-recipes-staticphp-logs-${{ matrix.track }}-${{ matrix.platform }}-${{ github.run_id }}"
        ),
    );

    assert_snapshot!(summary, @r###"
    jobs=["validate", "build-php", "build-composer", "build-redis", "build-mysql", "build-postgres", "build-mailpit", "build-rustfs"]
    upload_steps=8
    archive_upload_paths=7
    manifest_upload_paths=7
    record_upload_paths=7
    recipe_track_envs=8
    track_upload_names=8
    staticphp_failure_logs=true
    "###);

    Ok(())
}

#[test]
fn artifact_recipes_track_all_expands_requested_track_sets() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = read_file(&workspace_root.join(".github/workflows/artifact-recipes.yml"))?;
    let summary = format!(
        "track_matrices={:?}\nvalidated_resource_tracks={:?}",
        track_matrices(&workflow),
        validated_resource_tracks(&workflow),
    );

    assert_snapshot!(summary, @r###"
    track_matrices=["track: ${{ fromJSON(inputs.track == 'all' && '[\"8.3\",\"8.4\",\"8.5\"]' || format('[\"{0}\"]', inputs.track)) }}", "track: ${{ fromJSON(inputs.track == 'all' && '[\"2\"]' || format('[\"{0}\"]', inputs.track)) }}", "track: ${{ fromJSON(inputs.track == 'all' && '[\"8.8\"]' || format('[\"{0}\"]', inputs.track)) }}", "track: ${{ fromJSON(inputs.track == 'all' && '[\"8.0\",\"8.4\",\"9.7\"]' || format('[\"{0}\"]', inputs.track)) }}", "track: ${{ fromJSON(inputs.track == 'all' && '[\"17\",\"18\"]' || format('[\"{0}\"]', inputs.track)) }}", "track: ${{ fromJSON(inputs.track == 'all' && '[\"1\"]' || format('[\"{0}\"]', inputs.track)) }}", "track: ${{ fromJSON(inputs.track == 'all' && '[\"1\"]' || format('[\"{0}\"]', inputs.track)) }}"]
    validated_resource_tracks=["all:all", "php:all | php:8.3 | php:8.4 | php:8.5", "composer:all | composer:2", "redis:all | redis:8.8", "mysql:all | mysql:8.0 | mysql:8.4 | mysql:9.7", "postgres:all | postgres:17 | postgres:18", "mailpit:all | mailpit:1", "rustfs:all | rustfs:1"]
    "###);

    Ok(())
}

#[test]
fn artifact_publication_defaults_to_preview_native_platform_gate() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = read_file(&workspace_root.join(".github/workflows/artifact-publication.yml"))?;
    let summary = format!(
        "required_native_platforms_default={}\npasses_required_native_platforms={}\nvalidates_required_native_platforms={}\ntrims_required_native_platforms={}\nrejects_empty_required_native_platforms={}",
        input_default(&workflow, "required_native_platforms").unwrap_or(""),
        workflow.contains("--required-native-platform"),
        workflow.contains("unsupported required native platform"),
        workflow
            .matches("trimmed_platform=$(trim_required_platform \"$platform\")")
            .count(),
        workflow.contains("required native platform entries must be non-empty"),
    );

    assert_snapshot!(summary, @r###"
    required_native_platforms_default=darwin-arm64
    passes_required_native_platforms=true
    validates_required_native_platforms=true
    trims_required_native_platforms=2
    rejects_empty_required_native_platforms=true
    "###);

    Ok(())
}

#[test]
fn app_workflows_use_repository_r2_configuration() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let release = read_optional_file(&workspace_root.join(APP_RELEASE_WORKFLOW_PATH))?;
    let publication = read_optional_file(&workspace_root.join(APP_PUBLICATION_WORKFLOW_PATH))?;
    let release_workflow = release.as_deref().unwrap_or("");
    let publication_workflow = publication.as_deref().unwrap_or("");
    let summary = format!(
        "release_workflow_exists={}\npublication_workflow_exists={}\nrelease_name={}\npublication_name={}\nrelease_uses_r2_public_base_url_var={}\npublication_uses_r2_bucket_var={}\npublication_uses_r2_public_base_url_var={}\npublication_uses_cloudflare_account_secret={}\npublication_uses_r2_access_key_secret={}\npublication_uses_r2_secret_access_key_secret={}\npublication_derives_r2_endpoint_from_secret={}\npublication_hardcodes_staging_bucket={}\npublication_hardcodes_staging_public_base_url={}",
        release.is_some(),
        publication.is_some(),
        workflow_name(release_workflow).unwrap_or(""),
        workflow_name(publication_workflow).unwrap_or(""),
        release_workflow.contains("${{ vars.R2_PUBLIC_BASE_URL }}"),
        publication_workflow.contains("${{ vars.R2_BUCKET }}"),
        publication_workflow.contains("${{ vars.R2_PUBLIC_BASE_URL }}"),
        publication_workflow.contains("${{ secrets.CLOUDFLARE_ACCOUNT_ID }}"),
        publication_workflow.contains("${{ secrets.R2_ACCESS_KEY_ID }}"),
        publication_workflow.contains("${{ secrets.R2_SECRET_ACCESS_KEY }}"),
        publication_workflow
            .contains("https://${{ secrets.CLOUDFLARE_ACCOUNT_ID }}.r2.cloudflarestorage.com"),
        publication_workflow.contains("pv-staging"),
        publication_workflow.contains("artifacts-staging.pv.prvious.dev"),
    );

    assert_snapshot!(summary, @r#"
    release_workflow_exists=true
    publication_workflow_exists=true
    release_name=PV App Release
    publication_name=PV App Publication
    release_uses_r2_public_base_url_var=true
    publication_uses_r2_bucket_var=true
    publication_uses_r2_public_base_url_var=true
    publication_uses_cloudflare_account_secret=true
    publication_uses_r2_access_key_secret=true
    publication_uses_r2_secret_access_key_secret=true
    publication_derives_r2_endpoint_from_secret=true
    publication_hardcodes_staging_bucket=false
    publication_hardcodes_staging_public_base_url=false
    "#);

    Ok(())
}

#[test]
fn app_release_workflow_builds_native_binaries_and_handoff_artifacts() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let release = read_optional_file(&workspace_root.join(APP_RELEASE_WORKFLOW_PATH))?;
    let release_workflow = release.as_deref().unwrap_or("");
    let summary = format!(
        "release_workflow_exists={}\nrelease_name={}\njobs={:?}\nplatform_matrices={:?}\nuses_native_macos_runners={}\nmanifest_url_env={}\nuses_r2_public_base_url_var={}\nhardcodes_staging_bucket={}\nhardcodes_staging_public_base_url={}\nwrite_app_release_record_command={}\napp_manifest_command={}\napp_installer_command={}\napp_binary_object_key_refs={}\napp_record_object_key_refs={}\nupload_steps={}\nuploads_binaries={}\nuploads_records={}\nuploads_manifest={}\nuploads_installer={}",
        release.is_some(),
        workflow_name(release_workflow).unwrap_or(""),
        workflow_job_ids(release_workflow),
        platform_matrices(release_workflow),
        release_workflow.contains("macos-14") && release_workflow.contains("macos-15-intel"),
        release_workflow.contains("PV_DEFAULT_APP_UPDATE_MANIFEST_URL")
            && release_workflow.contains("${{ vars.R2_PUBLIC_BASE_URL }}/pv-app-manifest.json"),
        release_workflow.contains("${{ vars.R2_PUBLIC_BASE_URL }}"),
        release_workflow.contains("pv-staging"),
        release_workflow.contains("artifacts-staging.pv.prvious.dev"),
        release_workflow.contains("write-app-release-record"),
        release_workflow.contains("generate-app-manifest"),
        release_workflow.contains("generate-app-installer"),
        app_binary_object_key_reference_present(release_workflow),
        app_record_object_key_reference_present(release_workflow),
        uses_action_references(release_workflow, "actions/upload-artifact").len(),
        release_workflow.contains("${{ runner.temp }}/pv-app-release-stage/pv/${{ needs.prepare-release.outputs.version }}/pv-darwin-arm64")
            && release_workflow.contains("${{ runner.temp }}/pv-app-release-stage/pv/${{ needs.prepare-release.outputs.version }}/pv-darwin-amd64"),
        release_workflow.contains("${{ runner.temp }}/pv-app-release-stage/pv/records/${{ needs.prepare-release.outputs.version }}/*.json"),
        release_workflow.contains("${{ runner.temp }}/pv-app-release-stage/pv-app-manifest.json"),
        release_workflow.contains("${{ runner.temp }}/pv-app-release-stage/install.sh"),
    );

    assert_snapshot!(summary, @r#"
    release_workflow_exists=true
    release_name=PV App Release
    jobs=["prepare-release", "build-app", "generate-release"]
    platform_matrices=["platform: [darwin-arm64, darwin-amd64]"]
    uses_native_macos_runners=true
    manifest_url_env=true
    uses_r2_public_base_url_var=true
    hardcodes_staging_bucket=false
    hardcodes_staging_public_base_url=false
    write_app_release_record_command=true
    app_manifest_command=true
    app_installer_command=true
    app_binary_object_key_refs=true
    app_record_object_key_refs=true
    upload_steps=2
    uploads_binaries=true
    uploads_records=true
    uploads_manifest=true
    uploads_installer=true
    "#);

    Ok(())
}

#[test]
fn app_publication_writes_app_stable_entrypoints() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let publication = read_optional_file(&workspace_root.join(APP_PUBLICATION_WORKFLOW_PATH))?;
    let publication_workflow = publication.as_deref().unwrap_or("");
    let summary = format!(
        "publication_workflow_exists={}\nstable_app_manifest_key_present={}\nstable_installer_key_present={}\nmanaged_resource_key_references={:?}",
        publication.is_some(),
        stable_key_reference_present(publication_workflow, "pv-app-manifest.json"),
        stable_key_reference_present(publication_workflow, "install.sh"),
        managed_resource_key_references(publication_workflow),
    );

    assert_snapshot!(summary, @r#"
    publication_workflow_exists=true
    stable_app_manifest_key_present=true
    stable_installer_key_present=true
    managed_resource_key_references=[]
    "#);

    Ok(())
}

#[test]
fn app_publication_uses_immutable_upload_checks_for_app_objects() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let publication = read_optional_file(&workspace_root.join(APP_PUBLICATION_WORKFLOW_PATH))?;
    let publication_workflow = publication.as_deref().unwrap_or("");
    let summary = format!(
        "publication_workflow_exists={}\nuses_stage_app_publication_command={}\ndefines_immutable_upload_helper={}\nuses_if_none_match={}\nhandles_precondition_failed={}\napp_binary_object_key_refs={}\napp_record_object_key_refs={}\nversioned_manifest_object_key_refs={}\nversioned_installer_object_key_refs={}",
        publication.is_some(),
        publication_workflow.contains("stage-app-publication"),
        publication_workflow.contains("upload_immutable_object()"),
        publication_workflow.contains("--if-none-match '*'"),
        publication_workflow.contains("PreconditionFailed"),
        app_binary_object_key_reference_present(publication_workflow),
        publication_workflow.contains("pv/records/"),
        versioned_generated_artifact_reference_present(
            publication_workflow,
            "pv-app-manifest.json",
        ),
        versioned_generated_artifact_reference_present(publication_workflow, "install.sh"),
    );

    assert_snapshot!(summary, @r#"
    publication_workflow_exists=true
    uses_stage_app_publication_command=true
    defines_immutable_upload_helper=true
    uses_if_none_match=true
    handles_precondition_failed=true
    app_binary_object_key_refs=true
    app_record_object_key_refs=true
    versioned_manifest_object_key_refs=true
    versioned_installer_object_key_refs=true
    "#);

    Ok(())
}

#[test]
fn app_workflows_pin_actions_and_disable_checkout_credentials() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let release = read_optional_file(&workspace_root.join(APP_RELEASE_WORKFLOW_PATH))?;
    let publication = read_optional_file(&workspace_root.join(APP_PUBLICATION_WORKFLOW_PATH))?;
    let combined = format!(
        "{}\n{}",
        release.as_deref().unwrap_or(""),
        publication.as_deref().unwrap_or("")
    );
    let summary = format!(
        "unpinned_uses={:?}\ncheckout_persist_credentials_false_count={}",
        unpinned_uses_references(&combined),
        combined.matches("persist-credentials: false").count(),
    );

    assert_snapshot!(summary, @r#"
    unpinned_uses=[]
    checkout_persist_credentials_false_count=4
    "#);

    Ok(())
}

#[test]
fn app_publication_regenerates_entrypoints_and_validates_current_stable() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let publication = read_optional_file(&workspace_root.join(APP_PUBLICATION_WORKFLOW_PATH))?;
    let publication_workflow = publication.as_deref().unwrap_or("");
    let summary = format!(
        "passes_base_url={}\npasses_current_app_manifest={}\npasses_app_manifest_path={}\npasses_installer_path={}\nfetches_current_stable={}",
        publication_workflow.contains("--base-url \"$R2_PUBLIC_BASE_URL\""),
        publication_workflow.contains("--current-app-manifest"),
        publication_workflow.contains("--app-manifest"),
        publication_workflow.contains("--installer"),
        publication_workflow.contains("current-pv-app-manifest.json"),
    );

    assert_snapshot!(summary, @r#"
    passes_base_url=true
    passes_current_app_manifest=true
    passes_app_manifest_path=false
    passes_installer_path=false
    fetches_current_stable=true
    "#);

    Ok(())
}

#[test]
fn app_publication_retries_matching_immutable_uploads() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let publication = read_optional_file(&workspace_root.join(APP_PUBLICATION_WORKFLOW_PATH))?;
    let publication_workflow = publication.as_deref().unwrap_or("");
    let summary = format!(
        "downloads_existing_immutable={}\ncompares_existing_immutable={}\nrejects_different_existing_immutable={}",
        publication_workflow.contains("get-object"),
        publication_workflow.contains("cmp -s"),
        publication_workflow.contains("different content"),
    );

    assert_snapshot!(summary, @r#"
    downloads_existing_immutable=true
    compares_existing_immutable=true
    rejects_different_existing_immutable=true
    "#);

    Ok(())
}

#[test]
fn app_publication_publishes_stable_installer_before_manifest() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let publication = read_optional_file(&workspace_root.join(APP_PUBLICATION_WORKFLOW_PATH))?;
    let publication_workflow = publication.as_deref().unwrap_or("");
    let installer_index = publication_workflow.find("- name: Publish stable installer");
    let manifest_index = publication_workflow.find("- name: Publish stable app manifest");
    let summary = format!(
        "installer_step_present={}\nmanifest_step_present={}\ninstaller_before_manifest={}",
        installer_index.is_some(),
        manifest_index.is_some(),
        installer_index
            .zip(manifest_index)
            .map(|(installer_index, manifest_index)| installer_index < manifest_index)
            .unwrap_or(false),
    );

    assert_snapshot!(summary, @r#"
    installer_step_present=true
    manifest_step_present=true
    installer_before_manifest=true
    "#);

    Ok(())
}

fn input_default<'a>(workflow: &'a str, input: &str) -> Option<&'a str> {
    let input_header = format!("      {input}:");
    let mut in_input = false;

    for line in workflow.lines() {
        if line == input_header {
            in_input = true;
            continue;
        }

        if in_input && line.starts_with("      ") && !line.starts_with("        ") {
            return None;
        }

        if in_input {
            let Some(default_value) = line.strip_prefix("        default: ") else {
                continue;
            };
            return default_value.trim_matches('"').into();
        }
    }

    None
}

fn unpinned_uses_references(workflow: &str) -> Vec<&str> {
    workflow
        .lines()
        .filter_map(|line| line.trim().strip_prefix("uses: "))
        .filter(|reference| !uses_reference_is_pinned(reference))
        .collect()
}

fn uses_reference_is_pinned(reference: &str) -> bool {
    reference
        .rsplit_once('@')
        .map(|(_, revision)| {
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .unwrap_or(false)
}

fn uses_action_references<'a>(workflow: &'a str, action: &str) -> Vec<&'a str> {
    workflow
        .lines()
        .filter_map(|line| line.trim().strip_prefix("uses: "))
        .filter(|reference| {
            reference
                .split_once('@')
                .map(|(repository, _)| repository == action)
                .unwrap_or(false)
        })
        .collect()
}

fn input_description<'a>(workflow: &'a str, input: &str) -> Option<&'a str> {
    let input_header = format!("      {input}:");
    let mut in_input = false;

    for line in workflow.lines() {
        if line == input_header {
            in_input = true;
            continue;
        }

        if in_input && line.starts_with("      ") && !line.starts_with("        ") {
            return None;
        }

        if in_input {
            let Some(description) = line.strip_prefix("        description: ") else {
                continue;
            };
            return description.trim_matches('"').into();
        }
    }

    None
}

fn workflow_name(workflow: &str) -> Option<&str> {
    workflow
        .lines()
        .find_map(|line| line.strip_prefix("name: "))
        .map(|name| name.trim_matches('"'))
}

fn platform_matrices(workflow: &str) -> Vec<&str> {
    workflow
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with("platform: ${{ fromJSON(")
                || line.starts_with("platform: [")
                || line == "platform: [any]"
            {
                Some(line)
            } else {
                None
            }
        })
        .collect()
}

fn track_matrices(workflow: &str) -> Vec<&str> {
    workflow
        .lines()
        .filter(|line| line.trim_start().starts_with("track: ${{ fromJSON("))
        .map(str::trim)
        .collect()
}

fn validated_resource_tracks(workflow: &str) -> Vec<&str> {
    workflow
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.contains(':') && line.ends_with(") ;;") {
                Some(line.trim_end_matches(") ;;"))
            } else {
                None
            }
        })
        .filter(|line| line.contains('|') || *line == "all:all")
        .collect()
}

fn workflow_job_ids(workflow: &str) -> Vec<&str> {
    let mut in_jobs = false;
    let mut job_ids = Vec::new();

    for line in workflow.lines() {
        if line == "jobs:" {
            in_jobs = true;
            continue;
        }

        if !in_jobs {
            continue;
        }

        if !line.is_empty() && !line.starts_with("  ") {
            break;
        }

        let Some(candidate) = line.strip_prefix("  ") else {
            continue;
        };
        if candidate.starts_with(' ') {
            continue;
        }
        let Some(job_id) = candidate.strip_suffix(':') else {
            continue;
        };
        job_ids.push(job_id);
    }

    job_ids
}

fn stable_key_reference_present(workflow: &str, object_key: &str) -> bool {
    workflow.lines().any(|line| {
        let line = line.trim();
        line.contains(object_key)
            && (line.contains("STABLE")
                || line.contains("stable_")
                || line.contains("stable-")
                || line.contains(".stable"))
    })
}

fn managed_resource_key_references(workflow: &str) -> Vec<&str> {
    workflow
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with('#')
                || line.contains("pv-app-manifest.json")
                || line.contains("pv/records/")
                || line.contains("pv/manifests/")
            {
                return None;
            }

            if line.contains("manifest.json")
                || line.contains("resources/")
                || line.contains("records/")
                || line.contains("revocations/")
            {
                Some(line)
            } else {
                None
            }
        })
        .collect()
}

fn app_binary_object_key_reference_present(workflow: &str) -> bool {
    workflow.contains("pv/")
        && workflow.contains("pv-darwin-arm64")
        && workflow.contains("pv-darwin-amd64")
}

fn versioned_generated_artifact_reference_present(workflow: &str, artifact: &str) -> bool {
    workflow.contains("pv/manifests/runs") && workflow.contains(artifact)
}

fn app_record_object_key_reference_present(workflow: &str) -> bool {
    workflow.contains("pv/records/") && workflow.contains("pv-${{ matrix.platform }}.json")
}

fn path_exists(path: &Utf8Path) -> bool {
    path.exists()
}

fn read_optional_file(path: &Utf8Path) -> Result<Option<String>> {
    if !path_exists(path) {
        return Ok(None);
    }

    read_file(path).map(Some)
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read workflow fixtures directly"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}
