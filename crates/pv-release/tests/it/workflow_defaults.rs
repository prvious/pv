//! Runs scripts committed inside release workflows, with only their external calls replaced.

use anyhow::{Context, Result};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_snapshot;

#[expect(
    clippy::disallowed_types,
    reason = "workflow tests execute the release workflow's shell steps"
)]
type StdCommand = std::process::Command;

const APP_RELEASE_WORKFLOW_PATH: &str = ".github/workflows/app-release.yml";

#[cfg(target_os = "macos")]
#[test]
fn app_release_workflow_compares_helper_versions_with_macos_awk() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = read_file(&workspace_root.join(APP_RELEASE_WORKFLOW_PATH))?;
    let build_step = workflow_step(&workflow, "Build app and privileged helper binaries")
        .context("missing app build step")?;
    let start = build_step.find("'BEGIN {").context("missing awk program")? + 1;
    let end = build_step[start..]
        .find("}')")
        .context("missing awk program end")?
        + start
        + 1;
    let program = &build_step[start..end];
    let mut results = Vec::new();

    for (current, candidate) in [
        ("1.0.0", "1.0.0"),
        ("1.0.0", "1.0.1"),
        ("1.0.9", "1.0.10"),
        ("1.1.0", "1.0.9"),
    ] {
        let output = StdCommand::new("/usr/bin/awk")
            .args(["-v", &format!("current={current}")])
            .args(["-v", &format!("candidate={candidate}")])
            .arg(program)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "awk rejected the helper comparison: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        results.push(format!(
            "{current} -> {candidate}: {}",
            String::from_utf8(output.stdout)?.trim()
        ));
    }

    assert_snapshot!(results.join("\n"), @"
    1.0.0 -> 1.0.0: same
    1.0.0 -> 1.0.1: newer
    1.0.9 -> 1.0.10: newer
    1.1.0 -> 1.0.9: older
    ");

    Ok(())
}

#[test]
fn app_publication_readback_requires_both_exact_entrypoints() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = read_file(&workspace_root.join(APP_RELEASE_WORKFLOW_PATH))?;
    let step = workflow_step(&workflow, "Verify published app entrypoints")
        .context("app publication readback step is missing")?;
    let (_, script) = step
        .split_once("run: |\n")
        .context("readback script is missing")?;
    let script = script
        .lines()
        .map(|line| line.strip_prefix("          ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    // Only the HTTP transport is replaced; execute the committed comparison and summary code.
    let transport = r#"
curl() {
  if [ "$READBACK_FAILURE" = true ]; then return 22; fi
  local output url
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --output) output=$2; shift 2 ;;
      https://*) url=$1; shift ;;
      *) shift ;;
    esac
  done
  cp "$READBACK_FIXTURES/${url##*/}" "$output"
}
"#;
    let mut outcomes = Vec::new();
    for (case, stale_entrypoint, http_failure) in [
        ("matching", None, false),
        ("stale manifest", Some("pv-app-manifest.json"), false),
        ("stale installer", Some("install.sh"), false),
        ("HTTP failure", None, true),
    ] {
        let temp = tempdir()?;
        let stage = temp.path().join("pv-app-publication-stage");
        let public = temp.path().join("public");
        for (name, content) in [
            ("pv-app-manifest.json", "{\"version\":\"0.2.9\"}"),
            ("install.sh", "installer"),
        ] {
            write_file(&stage.join(name), content)?;
            write_file(&public.join(name), content)?;
        }
        if let Some(entrypoint) = stale_entrypoint {
            write_file(&public.join(entrypoint), "stale")?;
        }
        let summary = temp.path().join("summary");
        let output = StdCommand::new("bash")
            .args(["-c", &format!("{transport}\n{script}")])
            .env("RUNNER_TEMP", temp.path())
            .env("READBACK_FIXTURES", &public)
            .env("READBACK_FAILURE", http_failure.to_string())
            .env("R2_PUBLIC_BASE_URL", "https://downloads.prvious.test/")
            .env("GITHUB_STEP_SUMMARY", &summary)
            .output()?;
        let success_summary = read_optional_file(&summary)?
            .unwrap_or_default()
            .contains("Published PV 0.2.9");
        outcomes.push(format!(
            "{case}: success={}, success_summary={success_summary}",
            output.status.success()
        ));
    }
    assert_snapshot!(outcomes.join("\n"), @"
    matching: success=true, success_summary=true
    stale manifest: success=false, success_summary=false
    stale installer: success=false, success_summary=false
    HTTP failure: success=false, success_summary=false
    ");
    Ok(())
}

fn workflow_step<'a>(workflow: &'a str, name: &str) -> Option<&'a str> {
    let header = format!("      - name: {name}\n");
    let start = workflow.find(&header)? + header.len();
    let remaining = &workflow[start..];
    let end = remaining
        .find("\n      - name: ")
        .unwrap_or(remaining.len());

    Some(&remaining[..end])
}

fn read_optional_file(path: &Utf8Path) -> Result<Option<String>> {
    if !path.exists() {
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

#[expect(
    clippy::disallowed_methods,
    reason = "workflow test writes isolated publication readback fixtures"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(std::fs::write(path, content)?)
}
