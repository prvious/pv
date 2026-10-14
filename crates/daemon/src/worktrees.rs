//! Git worktree discovery: PV reads a source Project's `.git/worktrees/` directly, without running
//! `git`, and links each linked worktree as its own Project. DESIGN.md's Git Worktrees has the
//! rules.

use std::collections::BTreeSet;
use std::io;

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use config::ProjectConfigFile;
use state::{
    Database, DiscoveredProjectRecord, LinkDiscoveredProjectInput, ProjectMode, ProjectRecord,
    PvPaths, StateError,
};

use crate::{DaemonError, structured_log};

/// The PV-owned file in a worktree's Git entry directory that holds its worktree ID.
const WORKTREE_ID_FILE: &str = "pv-id";
const CONFIG_FILE: &str = "pv.yml";

/// One change that brings linked Projects in line with the worktrees Git registers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorktreeChange {
    /// Links a worktree as a new Project, first writing a new worktree ID if it has none.
    Link {
        source_project_id: String,
        entry_dir: Utf8PathBuf,
        worktree_id: Option<String>,
        worktree_name: String,
        path: Utf8PathBuf,
    },
    /// Writes back the ID of a discovered Project whose ID file is gone, found by its path.
    RestoreId {
        entry_dir: Utf8PathBuf,
        worktree_id: String,
    },
    /// Follows `git worktree move`.
    Move {
        project_id: String,
        path: Utf8PathBuf,
    },
    /// Git no longer registers the worktree.
    Remove { project_id: String },
    /// Git no longer registers a worktree the user unlinked.
    ForgetOptOut {
        source_project_id: String,
        worktree_id: String,
    },
}

pub(crate) struct WorktreePlan {
    pub(crate) changes: Vec<WorktreeChange>,
    /// Sources whose worktrees couldn't all be read. Discovery changes nothing for them, so
    /// nothing unreadable ever counts as removed.
    pub(crate) skipped: Vec<(String, DaemonError)>,
}

/// A worktree Git registers, and where the source Project's path falls inside it.
struct Worktree {
    entry_dir: Utf8PathBuf,
    worktree_id: Option<String>,
    worktree_name: String,
    path: Utf8PathBuf,
}

/// Reads every source Project's worktrees and returns the changes discovery would make, without
/// changing anything.
pub(crate) fn plan_worktree_changes(database: &Database) -> Result<WorktreePlan, DaemonError> {
    let discovered = database.discovered_projects()?;
    let opt_outs = database.worktree_opt_outs()?;
    let discovered_ids = discovered
        .iter()
        .map(|project| project.project_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut plan = WorktreePlan {
        changes: Vec::new(),
        skipped: Vec::new(),
    };

    for source in database.projects()? {
        if discovered_ids.contains(source.id.as_str()) {
            continue;
        }
        let worktrees = match source_worktrees(&source.path) {
            Ok(Some(worktrees)) => worktrees,
            Ok(None) => continue,
            Err(error) => {
                plan.skipped.push((source.id, error));
                continue;
            }
        };
        let children = discovered
            .iter()
            .filter(|project| project.source_project_id == source.id)
            .collect::<Vec<_>>();
        let opted_out = opt_outs
            .iter()
            .filter(|opt_out| opt_out.source_project_id == source.id)
            .map(|opt_out| opt_out.worktree_id.as_str())
            .collect::<BTreeSet<_>>();
        plan_source_changes(
            database,
            &source,
            &worktrees,
            &children,
            &opted_out,
            &mut plan.changes,
        )?;
    }

    Ok(plan)
}

/// Applies discovery's changes. Runs inside system reconciliation, which holds the jobs lock. A
/// change that fails is logged and comes back on the next scan.
// ponytail: a change that keeps failing, such as an unwritable `.git` directory, makes every
// health scan request system reconciliation; give failed changes a backoff if that shows up.
pub(crate) fn discover_worktrees(paths: &PvPaths) -> Result<(), DaemonError> {
    let mut database = Database::open(paths)?;
    let plan = plan_worktree_changes(&database)?;
    for (project_id, error) in plan.skipped {
        structured_log::worktree_discovery_failed(paths, &project_id, &error.to_string());
    }
    for change in plan.changes {
        let subject = change_subject(&change).to_string();
        if let Err(error) = apply_change(paths, &mut database, change) {
            structured_log::worktree_discovery_failed(paths, &subject, &error.to_string());
        }
    }

    Ok(())
}

fn plan_source_changes(
    database: &Database,
    source: &ProjectRecord,
    worktrees: &[Worktree],
    children: &[&DiscoveredProjectRecord],
    opted_out: &BTreeSet<&str>,
    changes: &mut Vec<WorktreeChange>,
) -> Result<(), DaemonError> {
    let mut registered = BTreeSet::new();
    // Applied after the removals, so a re-created worktree can take the name its predecessor had.
    let mut worktree_changes = Vec::new();

    for worktree in worktrees {
        let child = match &worktree.worktree_id {
            Some(worktree_id) => children
                .iter()
                .find(|child| child.worktree_id == *worktree_id),
            None => children.iter().find(|child| child.path == worktree.path),
        };
        if let Some(child) = child {
            registered.insert(child.worktree_id.clone());
            if worktree.worktree_id.is_none() {
                worktree_changes.push(WorktreeChange::RestoreId {
                    entry_dir: worktree.entry_dir.clone(),
                    worktree_id: child.worktree_id.clone(),
                });
            } else if child.path != worktree.path && path_is_free(database, &worktree.path)? {
                worktree_changes.push(WorktreeChange::Move {
                    project_id: child.project_id.clone(),
                    path: worktree.path.clone(),
                });
            }
            continue;
        }
        if let Some(worktree_id) = &worktree.worktree_id {
            registered.insert(worktree_id.clone());
            if opted_out.contains(worktree_id.as_str()) {
                continue;
            }
        }
        if path_is_free(database, &worktree.path)? {
            worktree_changes.push(WorktreeChange::Link {
                source_project_id: source.id.clone(),
                entry_dir: worktree.entry_dir.clone(),
                worktree_id: worktree.worktree_id.clone(),
                worktree_name: worktree.worktree_name.clone(),
                path: worktree.path.clone(),
            });
        }
    }

    for child in children {
        if !registered.contains(&child.worktree_id) {
            changes.push(WorktreeChange::Remove {
                project_id: child.project_id.clone(),
            });
        }
    }
    for worktree_id in opted_out {
        if !registered.contains(*worktree_id) {
            changes.push(WorktreeChange::ForgetOptOut {
                source_project_id: source.id.clone(),
                worktree_id: (*worktree_id).to_string(),
            });
        }
    }
    changes.append(&mut worktree_changes);

    Ok(())
}

/// An existing directory no Project is linked at yet.
fn path_is_free(database: &Database, path: &Utf8Path) -> Result<bool, DaemonError> {
    Ok(state::fs::path_is_directory(path)? && database.project_by_path(path)?.is_none())
}

/// The worktrees of the repository whose main checkout holds `source_path`, or `None` when it
/// isn't a source: a missing directory, or one outside a main checkout.
fn source_worktrees(source_path: &Utf8Path) -> Result<Option<Vec<Worktree>>, DaemonError> {
    // A missing source changes nothing: its disk might just not be mounted.
    if !state::fs::path_is_directory(source_path)? {
        return Ok(None);
    }
    let Some((main_checkout, relative_path)) = main_checkout(source_path)? else {
        return Ok(None);
    };
    let mut worktrees = Vec::new();

    // Git deletes `worktrees/` with its last worktree, so a missing directory means none.
    for entry_dir in state::fs::read_dir_paths(&main_checkout.join(".git/worktrees"))? {
        if !state::fs::path_is_directory(&entry_dir)? {
            continue;
        }
        let gitdir = state::fs::read_to_string(&entry_dir.join("gitdir"))?;
        // `gitdir` names the checkout's `.git` file, relative to the entry directory when Git's
        // `worktree.useRelativePaths` is on.
        let checkout_git_file = normalize(&entry_dir.join(gitdir.trim()));
        let Some(checkout) = checkout_git_file.parent() else {
            return Err(invalid_entry(&entry_dir, "its gitdir names no checkout"));
        };
        let Some(worktree_name) = checkout.file_name() else {
            return Err(invalid_entry(&entry_dir, "its checkout has no name"));
        };
        let path = checkout.join(&relative_path);
        let path = if state::fs::path_is_directory(&path)? {
            state::fs::canonicalize(&path)?
        } else {
            path
        };

        worktrees.push(Worktree {
            worktree_id: read_worktree_id(&entry_dir)?,
            worktree_name: worktree_name.to_string(),
            entry_dir,
            path,
        });
    }

    Ok(Some(worktrees))
}

/// The main checkout holding `path` and `path` relative to it. The nearest `.git` decides: a
/// directory is a main checkout, and a file is a linked worktree or a submodule.
fn main_checkout(path: &Utf8Path) -> Result<Option<(Utf8PathBuf, Utf8PathBuf)>, DaemonError> {
    for ancestor in path.ancestors() {
        let git = ancestor.join(".git");
        if state::fs::path_is_directory(&git)? {
            let relative_path = path
                .strip_prefix(ancestor)
                .map_or_else(|_error| Utf8PathBuf::new(), Utf8Path::to_path_buf);
            return Ok(Some((ancestor.to_path_buf(), relative_path)));
        }
        if state::fs::path_entry_exists(&git)? {
            return Ok(None);
        }
    }

    Ok(None)
}

fn read_worktree_id(entry_dir: &Utf8Path) -> Result<Option<String>, DaemonError> {
    let path = entry_dir.join(WORKTREE_ID_FILE);
    if !state::fs::path_entry_exists(&path)? {
        return Ok(None);
    }
    let worktree_id = state::fs::read_to_string(&path)?.trim().to_string();
    if worktree_id.is_empty() || !worktree_id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(invalid_entry(
            entry_dir,
            "its pv-id file holds no worktree ID",
        ));
    }

    Ok(Some(worktree_id))
}

/// Resolves `.` and `..` without touching the filesystem, since the checkout may be gone.
fn normalize(path: &Utf8Path) -> Utf8PathBuf {
    let mut normalized = Utf8PathBuf::new();
    for component in path.components() {
        match component {
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component),
        }
    }

    normalized
}

fn apply_change(
    paths: &PvPaths,
    database: &mut Database,
    change: WorktreeChange,
) -> Result<(), DaemonError> {
    match change {
        WorktreeChange::Link {
            source_project_id,
            entry_dir,
            worktree_id,
            worktree_name,
            path,
        } => {
            let worktree_id = match worktree_id {
                Some(worktree_id) => worktree_id,
                None => write_new_worktree_id(&entry_dir)?,
            };
            let (path, config_path, mode) = match ProjectConfigFile::read_from_root(&path) {
                Ok(config_file) => {
                    let mode = if config_file.config.serve {
                        ProjectMode::Served
                    } else {
                        ProjectMode::ResourceOnly
                    };
                    (path, config_file.path, mode)
                }
                // Reconciliation reports the invalid config, as for any linked Project.
                Err(_error) => {
                    let config_path = path.join(CONFIG_FILE);
                    (path, config_path, ProjectMode::Served)
                }
            };
            database.link_discovered_project(LinkDiscoveredProjectInput {
                source_project_id,
                worktree_id,
                worktree_name,
                path,
                config_path,
                mode,
            })?;
        }
        WorktreeChange::RestoreId {
            entry_dir,
            worktree_id,
        } => {
            state::fs::write_new_file(
                &entry_dir.join(WORKTREE_ID_FILE),
                &format!("{worktree_id}\n"),
            )?;
        }
        WorktreeChange::Move { project_id, path } => {
            let config_path = match ProjectConfigFile::read_from_root(&path) {
                Ok(config_file) => config_file.path,
                Err(_error) => path.join(CONFIG_FILE),
            };
            database.move_discovered_project(&project_id, &path, &config_path)?;
        }
        WorktreeChange::Remove { project_id } => {
            match state::fs::delete_dir_all(&paths.project_tls_dir(&project_id)) {
                Ok(()) => {}
                Err(StateError::Filesystem { source, .. })
                    if source.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            database.remove_discovered_project(&project_id)?;
        }
        WorktreeChange::ForgetOptOut {
            source_project_id,
            worktree_id,
        } => {
            database.forget_worktree_opt_out(&source_project_id, &worktree_id)?;
        }
    }

    Ok(())
}

fn write_new_worktree_id(entry_dir: &Utf8Path) -> Result<String, DaemonError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|source| DaemonError::UnexpectedProtocolResponse {
        reason: format!("failed to generate a worktree ID: {source}"),
    })?;
    let worktree_id = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    state::fs::write_new_file(
        &entry_dir.join(WORKTREE_ID_FILE),
        &format!("{worktree_id}\n"),
    )?;

    Ok(worktree_id)
}

fn change_subject(change: &WorktreeChange) -> &str {
    match change {
        WorktreeChange::Link { entry_dir, .. } | WorktreeChange::RestoreId { entry_dir, .. } => {
            entry_dir.as_str()
        }
        WorktreeChange::Move { project_id, .. } | WorktreeChange::Remove { project_id } => {
            project_id
        }
        WorktreeChange::ForgetOptOut {
            source_project_id, ..
        } => source_project_id,
    }
}

fn invalid_entry(entry_dir: &Utf8Path, reason: &str) -> DaemonError {
    StateError::Filesystem {
        path: entry_dir.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, reason.to_string()),
    }
    .into()
}
