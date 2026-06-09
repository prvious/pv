use anyhow::Result;
use camino::Utf8Path;
use insta::assert_snapshot;

#[test]
fn artifact_recipes_defaults_defer_staticphp_unstable_lanes() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = read_file(&workspace_root.join(".github/workflows/artifact-recipes.yml"))?;
    let summary = format!(
        "track_default={}\nplatform_default={}\nplatform_matrix={}\nstaticphp_comment_present={}\nstaticphp_work_cleanup_restores_write_permission={}",
        input_default(&workflow, "track").unwrap_or(""),
        input_default(&workflow, "platform").unwrap_or(""),
        platform_matrix(&workflow).unwrap_or(""),
        workflow.contains("StaticPHP v3"),
        workflow.contains("chmod -R u+w \"$PV_ARTIFACT_OUT_DIR/work\""),
    );

    assert_snapshot!(summary, @r###"
    track_default=8.4
    platform_default=darwin-arm64
    platform_matrix=platform: ${{ fromJSON(inputs.platform == 'all' && '["darwin-arm64"]' || format('["{0}"]', inputs.platform)) }}
    staticphp_comment_present=true
    staticphp_work_cleanup_restores_write_permission=true
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

fn platform_matrix(workflow: &str) -> Option<&str> {
    workflow
        .lines()
        .find(|line| line.trim_start().starts_with("platform: ${{ fromJSON("))
        .map(str::trim)
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read workflow fixtures directly"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}
