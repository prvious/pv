use anyhow::Result;
use camino::Utf8Path;
use insta::assert_snapshot;

const ARTIFACT_RECIPES_UPLOAD_PATHS: [&str; 3] = [
    "${{ runner.temp }}/pv-artifacts/*.tar.gz",
    "${{ runner.temp }}/pv-artifacts/manifest.json",
    "${{ runner.temp }}/pv-records/**/*.json",
];

#[test]
fn artifact_recipes_defaults_defer_staticphp_unstable_lanes() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = read_file(&workspace_root.join(".github/workflows/artifact-recipes.yml"))?;
    let summary = format!(
        "track_default={}\nplatform_default={}\nplatform_matrices={:?}\nstaticphp_comment_present={}\nstaticphp_work_cleanup_restores_write_permission={}",
        input_default(&workflow, "track").unwrap_or(""),
        input_default(&workflow, "platform").unwrap_or(""),
        platform_matrices(&workflow),
        workflow.contains("StaticPHP v3"),
        workflow.contains("chmod -R u+w \"$PV_ARTIFACT_OUT_DIR/work\""),
    );

    assert_snapshot!(summary, @r###"
    track_default=all
    platform_default=all
    platform_matrices=["platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}", "platform: [any]", "platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}", "platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}", "platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}", "platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}", "platform: ${{ fromJSON(inputs.platform == 'all' && '[\"darwin-arm64\"]' || format('[\"{0}\"]', inputs.platform)) }}"]
    staticphp_comment_present=true
    staticphp_work_cleanup_restores_write_permission=true
    "###);

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
            "pv-artifact-recipes-staticphp-logs-${{ matrix.platform }}-${{ github.run_id }}"
        ),
    );

    assert_snapshot!(summary, @r###"
    jobs=["validate", "build-php", "build-composer", "build-redis", "build-mysql", "build-postgres", "build-mailpit", "build-rustfs"]
    upload_steps=8
    archive_upload_paths=7
    manifest_upload_paths=7
    record_upload_paths=7
    recipe_track_envs=8
    track_upload_names=7
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
        "required_native_platforms_default={}\npasses_required_native_platforms={}\nvalidates_required_native_platforms={}",
        input_default(&workflow, "required_native_platforms").unwrap_or(""),
        workflow.contains("--required-native-platform"),
        workflow.contains("unsupported required native platform"),
    );

    assert_snapshot!(summary, @r###"
    required_native_platforms_default=darwin-arm64
    passes_required_native_platforms=true
    validates_required_native_platforms=true
    "###);

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

fn platform_matrices(workflow: &str) -> Vec<&str> {
    workflow
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with("platform: ${{ fromJSON(") || line == "platform: [any]" {
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

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read workflow fixtures directly"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}
