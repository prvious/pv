# PR 22A Artifact Manifest Endpoint Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Centralize the Managed Resource artifact manifest endpoint so CLI and daemon production paths use the same compiled default, while staging builds can opt into the staging endpoint at build time.

**Architecture:** Put the compiled default endpoint in the `resources` crate because both CLI and daemon already depend on it and the endpoint belongs to Managed Resource artifact discovery. Preserve CLI fake-environment URL injection and explicit daemon test-catalog injection, but remove duplicate production/default constants from CLI command modules and daemon catalog construction.

**Tech Stack:** Rust 2024 workspace, Cargo build script rerun hints, `option_env!`, `cargo nextest`, `cargo insta`.

---

### Task 1: Design Contract

**Files:**
- Modify: `DESIGN.md`

- [x] **Step 1: Add the compile-time endpoint policy**

Insert this near the Managed Resource artifact manifest/distribution section:

~~~markdown
The Managed Resource artifact manifest endpoint is a property of the built PV binary in v1. Production/default builds use PV's stable/default artifact manifest endpoint. Maintainer staging builds may override that compiled default by setting `PV_DEFAULT_ARTIFACT_MANIFEST_URL` at build time, for example:

```sh
PV_DEFAULT_ARTIFACT_MANIFEST_URL=https://artifacts-staging.pv.prvious.dev/manifest.json cargo build --release
```

PV v1 does not expose runtime/user-facing artifact manifest selection through CLI flags such as `--channel` or `--manifest-url`, config files, shell environment variables, LaunchAgent environment variables, shell profile edits, installer channel parameters, or database state. Runtime/user-facing manifest selection could redirect PV to an unintended artifact manifest and could cause the CLI and daemon to disagree about which manifest owns Managed Resource artifacts. Tests may still inject manifest URLs through test-only seams.
~~~

- [x] **Step 2: Verify the design text**

Run:

```bash
rg -n "PV_DEFAULT_ARTIFACT_MANIFEST_URL|--manifest-url|LaunchAgent environment variables" DESIGN.md
```

Expected: the new design text is present and no surrounding text promises runtime/user-facing channel selection.

### Task 2: Shared Compiled Endpoint Helper

**Files:**
- Create: `crates/resources/build.rs`
- Create: `crates/resources/src/endpoint.rs`
- Modify: `crates/resources/src/lib.rs`

- [x] **Step 1: Write the failing endpoint tests**

Create `crates/resources/src/endpoint.rs` with tests that reference the helper before it exists:

```rust
pub const ARTIFACT_MANIFEST_URL_BUILD_ENV: &str = "PV_DEFAULT_ARTIFACT_MANIFEST_URL";
pub const STABLE_ARTIFACT_MANIFEST_URL: &str = "https://artifacts.prvious.test/manifest.json";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_artifact_manifest_url_matches_compiled_value() {
        let expected = option_env!("PV_DEFAULT_ARTIFACT_MANIFEST_URL")
            .unwrap_or(STABLE_ARTIFACT_MANIFEST_URL);

        assert_eq!(default_artifact_manifest_url(), expected);
    }

    #[test]
    fn stable_artifact_manifest_url_is_the_current_default_endpoint() {
        assert_eq!(
            STABLE_ARTIFACT_MANIFEST_URL,
            "https://artifacts.prvious.test/manifest.json"
        );
    }
}
```

Expose the module in `crates/resources/src/lib.rs`:

```rust
pub mod endpoint;
pub use endpoint::{
    ARTIFACT_MANIFEST_URL_BUILD_ENV, STABLE_ARTIFACT_MANIFEST_URL,
    default_artifact_manifest_url,
};
```

- [x] **Step 2: Run the red test**

Run:

```bash
cargo nextest run -p resources -E 'test(default_artifact_manifest_url_matches_compiled_value)' --all-features --locked
```

Expected: FAIL because `default_artifact_manifest_url` is not implemented.

- [x] **Step 3: Implement the minimal helper**

Add this function to `crates/resources/src/endpoint.rs`:

```rust
pub fn default_artifact_manifest_url() -> &'static str {
    option_env!("PV_DEFAULT_ARTIFACT_MANIFEST_URL").unwrap_or(STABLE_ARTIFACT_MANIFEST_URL)
}
```

Create `crates/resources/build.rs`:

```rust
fn main() {
    println!("cargo:rerun-if-env-changed=PV_DEFAULT_ARTIFACT_MANIFEST_URL");
}
```

- [x] **Step 4: Run the green tests**

Run:

```bash
cargo nextest run -p resources -E 'test(default_artifact_manifest_url_matches_compiled_value) | test(stable_artifact_manifest_url_is_the_current_default_endpoint)' --all-features --locked
```

Expected: PASS.

### Task 3: CLI Endpoint Construction

**Files:**
- Modify: `crates/cli/src/environment.rs`
- Modify: `crates/cli/src/commands/artifact_resource.rs`
- Modify: `crates/cli/src/commands/php.rs`
- Modify: `crates/cli/src/commands/composer.rs`

- [x] **Step 1: Write the failing CLI helper tests**

Add a private CLI helper in `crates/cli/src/environment.rs` and tests that call it before implementation:

```rust
pub(crate) fn artifact_manifest_url(environment: &impl Environment) -> String {
    environment
        .artifact_manifest_url()
        .unwrap_or_else(|| resources::default_artifact_manifest_url().to_string())
}
```

Add tests in `crates/cli/src/environment.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Debug, Default)]
    struct TestEnvironment {
        manifest_url: Option<String>,
        vars: BTreeMap<String, OsString>,
    }

    impl TestEnvironment {
        fn with_manifest_url(mut self, manifest_url: &str) -> Self {
            self.manifest_url = Some(manifest_url.to_string());
            self
        }

        fn with_var(mut self, key: &str, value: &str) -> Self {
            self.vars.insert(key.to_string(), OsString::from(value));
            self
        }
    }

    impl Environment for TestEnvironment {
        fn var_os(&self, key: &str) -> Option<OsString> {
            self.vars.get(key).cloned()
        }

        fn home_dir(&self) -> Option<PathBuf> {
            None
        }

        fn current_dir(&self) -> io::Result<PathBuf> {
            Ok(PathBuf::new())
        }

        fn current_exe(&self) -> io::Result<PathBuf> {
            Ok(PathBuf::new())
        }

        fn stdin_is_terminal(&self) -> bool {
            false
        }

        fn read_line(&self) -> io::Result<String> {
            Ok(String::new())
        }

        fn open_url(&self, _url: &str) -> io::Result<()> {
            Ok(())
        }

        fn artifact_manifest_url(&self) -> Option<String> {
            self.manifest_url.clone()
        }
    }

    #[test]
    fn artifact_manifest_url_uses_compiled_default_without_test_override() {
        let environment = TestEnvironment::default();

        assert_eq!(
            artifact_manifest_url(&environment),
            resources::default_artifact_manifest_url()
        );
    }

    #[test]
    fn artifact_manifest_url_preserves_test_injection() {
        let environment =
            TestEnvironment::default().with_manifest_url("https://fixtures.example.test/manifest.json");

        assert_eq!(
            artifact_manifest_url(&environment),
            "https://fixtures.example.test/manifest.json"
        );
    }

    #[test]
    fn artifact_manifest_url_ignores_runtime_environment_variables() {
        let environment = TestEnvironment::default().with_var(
            resources::ARTIFACT_MANIFEST_URL_BUILD_ENV,
            "https://runtime.example.test/manifest.json",
        );

        assert_eq!(
            artifact_manifest_url(&environment),
            resources::default_artifact_manifest_url()
        );
    }
}
```

- [x] **Step 2: Run the red CLI tests**

Run:

```bash
cargo nextest run -p cli -E 'test(artifact_manifest_url_)' --all-features --locked
```

Expected: FAIL until the helper exists and compiles.

- [x] **Step 3: Wire CLI command construction**

Remove local `DEFAULT_MANIFEST_URL` constants from:

```text
crates/cli/src/commands/artifact_resource.rs
crates/cli/src/commands/php.rs
crates/cli/src/commands/composer.rs
```

In each file, import the helper:

```rust
use crate::environment::{Environment, artifact_manifest_url};
```

Change each `resource_commands` function to:

```rust
fn resource_commands(paths: &PvPaths, environment: &impl Environment) -> ManagedResourceCommands {
    ManagedResourceCommands::new(
        paths.clone(),
        artifact_manifest_url(environment),
        target_platform(environment),
    )
}
```

- [x] **Step 4: Run the green CLI tests**

Run:

```bash
cargo nextest run -p cli -E 'test(artifact_manifest_url_)' --all-features --locked
```

Expected: PASS.

### Task 4: Daemon Catalog Defaults

**Files:**
- Modify: `crates/daemon/src/managed_resources/mod.rs`
- Modify: `crates/daemon/src/managed_resources/tests.rs`
- Modify: `crates/daemon/src/managed_resources/mysql_tests.rs`

- [x] **Step 1: Write the failing daemon catalog tests**

Add tests to `crates/daemon/src/managed_resources/tests.rs`:

```rust
#[test]
fn production_catalog_uses_compiled_artifact_manifest_endpoint() {
    let catalog = super::ManagedResourceRuntimeCatalog::production();

    assert_eq!(
        catalog.install_options.manifest_url,
        resources::default_artifact_manifest_url()
    );
}

#[test]
fn without_adapters_catalog_uses_compiled_artifact_manifest_endpoint() {
    let catalog = super::ManagedResourceRuntimeCatalog::without_adapters();

    assert_eq!(
        catalog.install_options.manifest_url,
        resources::default_artifact_manifest_url()
    );
}
```

- [x] **Step 2: Run the red daemon tests**

Run:

```bash
PV_DEFAULT_ARTIFACT_MANIFEST_URL=https://artifacts-staging.pv.prvious.dev/manifest.json cargo nextest run -p daemon -E 'test(catalog_uses_compiled_artifact_manifest_endpoint)' --all-features --locked
```

Expected: FAIL until the daemon catalog uses the shared compiled endpoint because the old daemon-local default still points at the stable endpoint.

- [x] **Step 3: Wire daemon production defaults**

Remove the daemon-local `DEFAULT_MANIFEST_URL` constant. In `ManagedResourceRuntimeCatalog::production()` and `ManagedResourceRuntimeCatalog::without_adapters()`, set:

```rust
manifest_url: resources::default_artifact_manifest_url().to_string(),
```

Replace test references to `super::DEFAULT_MANIFEST_URL` with:

```rust
resources::default_artifact_manifest_url()
```

Use `.to_string()` only where the field requires `String`.

- [x] **Step 4: Run the green daemon tests**

Run:

```bash
cargo nextest run -p daemon -E 'test(catalog_uses_compiled_artifact_manifest_endpoint)' --all-features --locked
PV_DEFAULT_ARTIFACT_MANIFEST_URL=https://artifacts-staging.pv.prvious.dev/manifest.json cargo nextest run -p daemon -E 'test(catalog_uses_compiled_artifact_manifest_endpoint)' --all-features --locked
```

Expected: both commands PASS.

### Task 5: Scope Guard And Verification

**Files:**
- Inspect: all changed files

- [x] **Step 1: Confirm duplicate defaults are gone**

Run:

```bash
rg -n "DEFAULT_MANIFEST_URL|PV_ARTIFACT_MANIFEST_URL|--manifest-url|--channel|EnvironmentVariables|artifact_manifest_url" crates/cli crates/daemon crates/resources crates/platform DESIGN.md docs/superpowers/plans/2026-06-10-pr-22a-artifact-manifest-endpoint.md
```

Expected: no duplicate production/default manifest constants, no runtime `PV_ARTIFACT_MANIFEST_URL`, no new CLI flags, and no LaunchAgent `EnvironmentVariables`.

- [x] **Step 2: Run focused tests**

Run:

```bash
cargo nextest run -p resources -E 'test(default_artifact_manifest_url_matches_compiled_value) | test(stable_artifact_manifest_url_is_the_current_default_endpoint)' --all-features --locked
cargo nextest run -p cli -E 'test(artifact_manifest_url_)' --all-features --locked
cargo nextest run -p daemon -E 'test(catalog_uses_compiled_artifact_manifest_endpoint)' --all-features --locked
```

Expected: PASS.

- [x] **Step 3: Run staging compile-time check**

Run:

```bash
PV_DEFAULT_ARTIFACT_MANIFEST_URL=https://artifacts-staging.pv.prvious.dev/manifest.json cargo nextest run -p resources -E 'test(default_artifact_manifest_url_matches_compiled_value)' --all-features --locked
```

Expected: PASS after Cargo rebuilds the `resources` crate with the build-time value.

- [x] **Step 4: Run repository quality gates**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo insta pending-snapshots --workspace
git diff --check
```

Expected: PASS, with no pending snapshots.

- [x] **Step 5: Commit, push, and open PR**

Run:

```bash
git status --short
git add DESIGN.md docs/superpowers/plans/2026-06-10-pr-22a-artifact-manifest-endpoint.md crates/resources/build.rs crates/resources/src/endpoint.rs crates/resources/src/lib.rs crates/cli/src/environment.rs crates/cli/src/commands/artifact_resource.rs crates/cli/src/commands/php.rs crates/cli/src/commands/composer.rs crates/daemon/src/managed_resources/mod.rs crates/daemon/src/managed_resources/tests.rs crates/daemon/src/managed_resources/mysql_tests.rs
git commit -m "feat(resources): centralize artifact manifest endpoint"
git push -u origin feat/pr22a-artifact-manifest-endpoint
gh pr create --title "feat(resources): centralize artifact manifest endpoint" --body-file /tmp/pr22a-artifact-manifest-endpoint-pr.md
```

Expected: branch pushed and GitHub PR opened for review.
