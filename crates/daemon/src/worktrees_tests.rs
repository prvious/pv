use anyhow::{Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use state::{Database, LinkProjectInput, ProjectRecord, PvPaths};

use crate::worktrees::{WorktreeChange, discover_worktrees, plan_worktree_changes};

/// A discovered Project's worktree ID, slug, hostname, and path under the fixture root.
type Discovered = (String, String, Option<String>, String);

/// A PV home and a code directory holding repositories in the layout Git 2.53 writes for
/// worktrees.
struct Fixture {
    _tempdir: Utf8TempDir,
    root: Utf8PathBuf,
    paths: PvPaths,
}

impl Fixture {
    fn new() -> Result<Self> {
        let tempdir = tempdir()?;
        // Discovery compares canonical paths, and macOS temporary directories sit behind a
        // symlink.
        let root = state::fs::canonicalize(tempdir.path())?;
        let paths = PvPaths::for_home(root.join("home"));

        Ok(Self {
            _tempdir: tempdir,
            root,
            paths,
        })
    }

    /// A main checkout: a directory holding a `.git` directory.
    fn main_checkout(&self, name: &str) -> Result<Utf8PathBuf> {
        let checkout = self.root.join(name);
        state::fs::ensure_user_dir(&checkout.join(".git"))?;

        Ok(checkout)
    }

    /// Registers a linked worktree the way `git worktree add` does: an entry directory under the
    /// main checkout's `.git/worktrees/` whose `gitdir` names the checkout's `.git` file.
    fn add_worktree(&self, main: &Utf8Path, entry: &str, checkout: &str) -> Result<Utf8PathBuf> {
        let entry_dir = main.join(".git/worktrees").join(entry);
        let checkout = self.root.join(checkout);
        state::fs::write_sensitive_file(&entry_dir.join("gitdir"), &format!("{checkout}/.git\n"))?;
        state::fs::write_sensitive_file(&checkout.join(".git"), &format!("gitdir: {entry_dir}\n"))?;

        Ok(entry_dir)
    }

    fn link(&self, path: &str, hostname: &str) -> Result<ProjectRecord> {
        let path = self.root.join(path);
        state::fs::ensure_user_dir(&path)?;

        Ok(Database::open(&self.paths)?
            .link_project(LinkProjectInput {
                original_path: path.clone(),
                primary_hostname: hostname.to_string(),
                config_path: path.join("pv.yml"),
                desired_php_track: None,
                path,
            })?
            .project)
    }

    fn discovered(&self) -> Result<Vec<Discovered>> {
        let database = Database::open(&self.paths)?;

        database
            .discovered_projects()?
            .into_iter()
            .map(|discovered| {
                let project = database
                    .project_by_id(&discovered.project_id)?
                    .ok_or_else(|| {
                        anyhow!("discovered Project {} is missing", discovered.project_id)
                    })?;
                Ok((
                    discovered.worktree_id,
                    project.slug,
                    project.primary_hostname,
                    project.path.strip_prefix(&self.root)?.to_string(),
                ))
            })
            .collect()
    }

    fn pending_changes(&self) -> Result<Vec<WorktreeChange>> {
        Ok(plan_worktree_changes(&Database::open(&self.paths)?)?.changes)
    }

    fn remove(&self, path: &Utf8Path) -> Result<()> {
        Ok(state::fs::delete_dir_all(path)?)
    }
}

fn worktree_id(entry_dir: &Utf8Path) -> Result<String> {
    Ok(state::fs::read_to_string(&entry_dir.join("pv-id"))?
        .trim()
        .to_string())
}

#[test]
fn discovery_links_follows_and_unlinks_worktrees() -> Result<()> {
    let fixture = Fixture::new()?;
    let repo = fixture.main_checkout("repo")?;
    fixture.link("repo", "repo.test")?;
    let entry = fixture.add_worktree(&repo, "feat", "worktrees/feat")?;

    discover_worktrees(&fixture.paths)?;
    let worktree_id = worktree_id(&entry)?;
    let linked = fixture.discovered()?;
    let settled = fixture.pending_changes()?;

    // `git worktree move` rewrites the entry's `gitdir` and moves the checkout.
    fixture.remove(&fixture.root.join("worktrees/feat"))?;
    fixture.add_worktree(&repo, "feat", "moved/feat")?;
    discover_worktrees(&fixture.paths)?;
    let moved = fixture.discovered()?;

    // `git worktree remove` deletes the checkout and the entry.
    fixture.remove(&fixture.root.join("moved/feat"))?;
    fixture.remove(&entry)?;
    discover_worktrees(&fixture.paths)?;
    let removed = fixture.discovered()?;

    let feat = |path: &str| {
        (
            worktree_id.clone(),
            "repo-feat".to_string(),
            Some("repo-feat.test".to_string()),
            path.to_string(),
        )
    };
    assert_eq!(linked, [feat("worktrees/feat")]);
    assert!(settled.is_empty(), "nothing left to change: {settled:?}");
    assert_eq!(moved, [feat("moved/feat")]);
    assert!(removed.is_empty());
    assert!(
        Database::open(&fixture.paths)?
            .project_by_path(&repo)?
            .is_some(),
        "the source stays linked"
    );

    Ok(())
}

#[test]
fn discovery_tells_a_recreated_worktree_from_one_that_lost_its_id() -> Result<()> {
    let fixture = Fixture::new()?;
    let repo = fixture.main_checkout("repo")?;
    fixture.link("repo", "repo.test")?;
    let entry = fixture.add_worktree(&repo, "feat", "worktrees/feat")?;
    discover_worktrees(&fixture.paths)?;
    let first_id = worktree_id(&entry)?;

    // Removed and added again while PV wasn't running: Git reuses the entry name, and the new
    // entry has no ID file.
    fixture.remove(&fixture.root.join("worktrees/feat"))?;
    fixture.remove(&entry)?;
    fixture.add_worktree(&repo, "feat", "worktrees/feat-again")?;
    discover_worktrees(&fixture.paths)?;
    let second_id = worktree_id(&entry)?;
    let recreated = fixture.discovered()?;

    // An entry that lost its ID file is the same worktree while its path still matches.
    state::fs::remove_file(&entry.join("pv-id"))?;
    discover_worktrees(&fixture.paths)?;
    let restored_id = worktree_id(&entry)?;

    assert_ne!(first_id, second_id);
    assert_eq!(
        recreated,
        [(
            second_id.clone(),
            "repo-feat-again".to_string(),
            Some("repo-feat-again.test".to_string()),
            "worktrees/feat-again".to_string()
        )]
    );
    assert_eq!(restored_id, second_id);
    assert_eq!(fixture.discovered()?, recreated);

    Ok(())
}

#[test]
fn discovery_maps_a_subfolder_source_and_skips_what_it_does_not_own() -> Result<()> {
    let fixture = Fixture::new()?;
    let repo = fixture.main_checkout("repo")?;
    fixture.link("repo/apps/web", "web.test")?;
    fixture.add_worktree(&repo, "feat", "worktrees/feat")?;
    state::fs::ensure_user_dir(&fixture.root.join("worktrees/feat/apps/web"))?;
    // This worktree's branch has no `apps/web` yet.
    fixture.add_worktree(&repo, "early", "worktrees/early")?;
    // The user linked this one by hand.
    fixture.add_worktree(&repo, "mine", "worktrees/mine")?;
    fixture.link("worktrees/mine/apps/web", "mine.test")?;
    // A Project inside a linked worktree, whose `.git` is a file, is never a source.
    fixture.link("worktrees/early", "early.test")?;

    discover_worktrees(&fixture.paths)?;
    let discovered = fixture.discovered()?;

    assert_eq!(
        discovered
            .into_iter()
            .map(|(_worktree_id, slug, _hostname, path)| (slug, path))
            .collect::<Vec<_>>(),
        [(
            "web-feat".to_string(),
            "worktrees/feat/apps/web".to_string()
        )]
    );
    assert!(fixture.pending_changes()?.is_empty());

    Ok(())
}

#[test]
fn discovery_changes_nothing_for_a_source_with_an_unreadable_entry() -> Result<()> {
    let fixture = Fixture::new()?;
    let repo = fixture.main_checkout("repo")?;
    fixture.link("repo", "repo.test")?;
    let feat = fixture.add_worktree(&repo, "feat", "worktrees/feat")?;
    discover_worktrees(&fixture.paths)?;
    let before = fixture.discovered()?;

    // Git removed `feat`, but a new entry has no `gitdir` yet, as while `git worktree add` runs.
    fixture.remove(&fixture.root.join("worktrees/feat"))?;
    fixture.remove(&feat)?;
    let new = repo.join(".git/worktrees/new");
    state::fs::ensure_user_dir(&new)?;
    discover_worktrees(&fixture.paths)?;
    let while_unreadable = fixture.discovered()?;

    // Git writes a relative `gitdir` when `worktree.useRelativePaths` is on.
    state::fs::write_sensitive_file(&new.join("gitdir"), "../../../../worktrees/new/.git\n")?;
    state::fs::write_sensitive_file(
        &fixture.root.join("worktrees/new/.git"),
        &format!("gitdir: {new}\n"),
    )?;
    discover_worktrees(&fixture.paths)?;
    let readable = fixture.discovered()?;

    assert_eq!(while_unreadable, before);
    assert_eq!(
        readable
            .into_iter()
            .map(|(_worktree_id, slug, _hostname, path)| (slug, path))
            .collect::<Vec<_>>(),
        [("repo-new".to_string(), "worktrees/new".to_string())]
    );

    Ok(())
}

#[test]
fn discovery_changes_nothing_while_an_entry_is_half_written_or_corrupt() -> Result<()> {
    let fixture = Fixture::new()?;
    let repo = fixture.main_checkout("repo")?;
    fixture.link("repo", "repo.test")?;
    let entry = fixture.add_worktree(&repo, "feat", "worktrees/feat")?;
    discover_worktrees(&fixture.paths)?;
    let before = fixture.discovered()?;
    let gitdir = state::fs::read_to_string(&entry.join("gitdir"))?;
    let worktree_id = worktree_id(&entry)?;

    // Git truncates `gitdir` before writing it during `git worktree move` or `repair`.
    for contents in ["", "/Users/me/Co"] {
        state::fs::write_sensitive_file(&entry.join("gitdir"), contents)?;
        discover_worktrees(&fixture.paths)?;
        assert_eq!(fixture.discovered()?, before, "gitdir {contents:?}");
    }
    state::fs::write_sensitive_file(&entry.join("gitdir"), &gitdir)?;
    // A `pv-id` PV would never write, such as one longer than a worktree ID can be.
    state::fs::write_sensitive_file(&entry.join("pv-id"), &"a".repeat(64))?;
    discover_worktrees(&fixture.paths)?;
    let after_corrupt_id = fixture.discovered()?;
    let skipped = plan_worktree_changes(&Database::open(&fixture.paths)?)?.skipped;
    state::fs::write_sensitive_file(&entry.join("pv-id"), &worktree_id)?;
    let after_repair = fixture.discovered()?;
    // Two entries holding one ID, as after copying a `pv-id` by hand.
    let copy = fixture.add_worktree(&repo, "copy", "worktrees/copy")?;
    discover_worktrees(&fixture.paths)?;
    let with_copy = fixture.discovered()?;
    state::fs::write_sensitive_file(&copy.join("pv-id"), &worktree_id)?;
    discover_worktrees(&fixture.paths)?;

    assert_eq!(after_corrupt_id, before);
    assert_eq!(skipped.len(), 1);
    assert_eq!(after_repair, before);
    assert_eq!(with_copy.len(), 2);
    assert_eq!(fixture.discovered()?, with_copy);

    Ok(())
}

#[test]
fn every_source_in_a_repository_links_its_part_of_a_new_worktree_in_one_run() -> Result<()> {
    let fixture = Fixture::new()?;
    let repo = fixture.main_checkout("repo")?;
    fixture.link("repo/apps/web", "web.test")?;
    fixture.link("repo/apps/api", "api.test")?;
    let entry = fixture.add_worktree(&repo, "feat", "worktrees/feat")?;
    for app in ["web", "api"] {
        state::fs::ensure_user_dir(&fixture.root.join("worktrees/feat/apps").join(app))?;
    }

    discover_worktrees(&fixture.paths)?;
    let worktree_id = worktree_id(&entry)?;

    assert_eq!(
        fixture
            .discovered()?
            .into_iter()
            .map(|(id, slug, _hostname, path)| (id == worktree_id, slug, path))
            .collect::<std::collections::BTreeSet<_>>(),
        [
            (
                true,
                "api-feat".to_string(),
                "worktrees/feat/apps/api".to_string()
            ),
            (
                true,
                "web-feat".to_string(),
                "worktrees/feat/apps/web".to_string()
            ),
        ]
        .into_iter()
        .collect()
    );

    Ok(())
}

#[test]
fn an_unlinked_worktree_stays_unlinked_until_git_drops_it() -> Result<()> {
    let fixture = Fixture::new()?;
    let repo = fixture.main_checkout("repo")?;
    let source = fixture.link("repo", "repo.test")?;
    let entry = fixture.add_worktree(&repo, "feat", "worktrees/feat")?;
    discover_worktrees(&fixture.paths)?;
    let Some((_worktree_id, slug, _hostname, _path)) = fixture.discovered()?.pop() else {
        return Err(anyhow!("feat wasn't discovered"));
    };
    let mut database = Database::open(&fixture.paths)?;
    let Some(feat) = database.project_by_slug(&slug)? else {
        return Err(anyhow!("feat has no Project"));
    };

    database.unlink_project(&feat.id)?;
    discover_worktrees(&fixture.paths)?;
    let after_unlink = (fixture.discovered()?, fixture.pending_changes()?);
    fixture.remove(&entry)?;
    discover_worktrees(&fixture.paths)?;
    let opt_outs = Database::open(&fixture.paths)?.worktree_opt_outs()?;

    assert_eq!(after_unlink, (Vec::new(), Vec::new()));
    assert!(
        opt_outs.is_empty(),
        "Git dropped the worktree: {opt_outs:?}"
    );
    assert!(
        Database::open(&fixture.paths)?
            .project_by_id(&source.id)?
            .is_some()
    );

    Ok(())
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn the_health_scan_requests_system_reconciliation_for_worktree_changes() -> Result<()> {
    let fixture = Fixture::new()?;
    let repo = fixture.main_checkout("repo")?;
    fixture.link("repo", "repo.test")?;
    fixture.add_worktree(&repo, "feat", "worktrees/feat")?;
    let catalog = std::sync::Arc::new(
        crate::managed_resources::ManagedResourceRuntimeCatalog::without_adapters()?,
    );

    let before = crate::health::scan_runtime_health(fixture.paths.clone(), Some(catalog.clone()))
        .await?
        .maintenance_scopes;
    discover_worktrees(&fixture.paths)?;
    let after = crate::health::scan_runtime_health(fixture.paths.clone(), Some(catalog.clone()))
        .await?
        .maintenance_scopes;
    // An entry without `gitdir` makes the source unreadable, which every scan reports.
    state::fs::ensure_user_dir(&repo.join(".git/worktrees/new"))?;
    let unreadable = crate::health::scan_runtime_health(fixture.paths.clone(), Some(catalog))
        .await?
        .errors;

    assert!(before.contains(&crate::ReconciliationScope::System));
    assert!(!after.contains(&crate::ReconciliationScope::System));
    assert!(
        unreadable.iter().any(|error| error.subject == "worktrees"),
        "unreadable source not reported: {:?}",
        unreadable
            .iter()
            .map(|error| &error.error)
            .collect::<Vec<_>>()
    );

    Ok(())
}
