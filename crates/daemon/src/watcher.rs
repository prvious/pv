//! Follows linked Projects' config files, and the Git worktree entries of worktree discovery
//! sources, through OS file events, so an idle daemon does no watcher work. DESIGN.md has the
//! rules.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use camino::{Utf8Path, Utf8PathBuf};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use state::{Database, ProjectConfigWatch, PvPaths, StateError, fs};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::reconciliation::{ReconciliationDebouncer, ReconciliationScope};
use crate::{DaemonError, structured_log};

const PREFERRED_CONFIG_FILE: &str = "pv.yml";
const ALTERNATE_CONFIG_FILE: &str = "pv.yaml";

/// A handle to the daemon's file watcher. Clones share one watcher.
#[derive(Clone)]
pub(crate) struct ProjectConfigWatcher {
    paths: PvPaths,
    state: Arc<Mutex<WatchState>>,
}

pub(crate) enum WatchMessage {
    Event(notify::Result<Event>),
    /// The watched paths changed, which restarts FSEvents' stream, so events may have been
    /// missed.
    Restarted,
}

struct WatchState {
    watcher: RecommendedWatcher,
    messages: UnboundedSender<WatchMessage>,
    watched: BTreeMap<Utf8PathBuf, RecursiveMode>,
    projects: BTreeMap<String, WatchedProject>,
}

struct WatchedProject {
    path: Utf8PathBuf,
    /// The canonical paths whose events change the config: `pv.yml` and `pv.yaml` in the Project
    /// directory, and the files they link to. Events name canonical paths.
    config_paths: BTreeSet<Utf8PathBuf>,
    config: ConfigModifiedTimes,
    /// For a discovery source, its repository's `.git` directory and the worktree entries last
    /// read there.
    worktrees: Option<(Utf8PathBuf, WorktreeEntries)>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ConfigModifiedTimes {
    preferred: Option<SystemTime>,
    alternate: Option<SystemTime>,
}

/// What worktree discovery would act on in a repository's `.git/worktrees/`: each entry, with the
/// modified times of its `gitdir` file, which Git rewrites when it moves the worktree, and its
/// `locked` file, which Git deletes once `git worktree add` has checked the worktree out. Git
/// rewrites an entry's other files, such as its index, for every command run in that worktree,
/// and FSEvents reports the entry directory itself with them.
type WorktreeEntries = BTreeMap<Utf8PathBuf, [Option<SystemTime>; 2]>;

impl ProjectConfigWatcher {
    /// Watches the Projects linked in `database`. The daemon calls this before it serves, so the
    /// first command it answers finds them watched.
    pub(crate) fn start(
        paths: PvPaths,
        database: &Database,
    ) -> Result<(Self, UnboundedReceiver<WatchMessage>), DaemonError> {
        let (messages, receiver) = unbounded_channel();
        let events = messages.clone();
        // notify calls this on its own thread, which a reload stops while it holds the watch
        // state, so it must never wait for that state.
        let watcher = notify::recommended_watcher(move |event| {
            let _result = events.send(WatchMessage::Event(event));
        })?;
        let watcher = Self {
            paths,
            state: Arc::new(Mutex::new(WatchState {
                watcher,
                messages,
                watched: BTreeMap::new(),
                projects: BTreeMap::new(),
            })),
        };
        watcher.watch(database.project_config_watches()?);
        // Startup reconciliation discovers the worktrees that exist now.
        let _scopes = lock_watch_state(&watcher.state).changes();

        Ok((watcher, receiver))
    }

    /// Re-reads the linked Projects from `pv.db` and follows them. The job queue calls this when
    /// it admits a job, after the command that asked for it committed its change, and when a job
    /// ends, after worktree discovery linked, moved, or unlinked Projects.
    pub(crate) fn reload(&self) {
        match load_project_config_watches(&self.paths) {
            Ok(Some(projects)) => self.watch(projects),
            Ok(None) => {}
            Err(error) => {
                structured_log::file_watch_failed(
                    &self.paths,
                    self.paths.db().as_str(),
                    &error.to_string(),
                );
            }
        }
    }

    /// Requests reconciliation for what each file event changed, until the daemon stops.
    pub(crate) async fn run(
        self,
        mut messages: UnboundedReceiver<WatchMessage>,
        debouncer: ReconciliationDebouncer,
    ) -> Result<(), DaemonError> {
        while let Some(message) = messages.recv().await {
            let watcher = self.clone();
            // Checking a config reads its modified time, which can block on a slow disk.
            let scopes = tokio::task::spawn_blocking(move || watcher.scopes(message)).await?;
            for scope in scopes {
                debouncer.request(scope).await;
            }
        }

        Ok(())
    }

    fn watch(&self, projects: Vec<ProjectConfigWatch>) {
        let mut state = lock_watch_state(&self.state);
        let state = &mut *state;
        let mut watched = BTreeMap::new();
        let mut watched_projects = BTreeMap::new();

        for project in projects {
            // A new Project's times are read before its directory is watched, so the check after
            // the restart catches a save in between.
            let (config, previous_worktrees) = match state.projects.remove(&project.project_id) {
                Some(watched) if watched.path == project.project_path => {
                    (watched.config, watched.worktrees)
                }
                _ => (config_modified_times(&project.project_path), None),
            };
            let mut config_paths = BTreeSet::new();
            let mut worktrees = None;
            if let Some(directory) = existing_directory(&self.paths, &project.project_path) {
                watched.insert(directory.clone(), RecursiveMode::NonRecursive);
                for file_name in [PREFERRED_CONFIG_FILE, ALTERNATE_CONFIG_FILE] {
                    let config_path = directory.join(file_name);
                    // Editing the file a config links to doesn't touch the link.
                    if let Ok(target) = fs::canonicalize(&config_path)
                        && target != config_path
                        && let Some(target_directory) = target.parent()
                    {
                        watched.insert(target_directory.to_path_buf(), RecursiveMode::NonRecursive);
                        config_paths.insert(target);
                    }
                    config_paths.insert(config_path);
                }
                if !project.discovered
                    && let Some(git) = repository_git_dir(&self.paths, &directory)
                {
                    // `.git` shows `worktrees/` appearing with the first worktree.
                    watched.insert(git.clone(), RecursiveMode::NonRecursive);
                    let worktrees_directory = git.join("worktrees");
                    if matches!(fs::path_is_directory(&worktrees_directory), Ok(true)) {
                        watched.insert(worktrees_directory, RecursiveMode::Recursive);
                    }
                    // A new source's worktrees are new to discovery too, even in a repository
                    // that another source already watches.
                    let entries = previous_worktrees
                        .filter(|(previous_git, _entries)| *previous_git == git)
                        .map_or_else(WorktreeEntries::new, |(_git, entries)| entries);
                    worktrees = Some((git, entries));
                }
            }
            watched_projects.insert(
                project.project_id,
                WatchedProject {
                    path: project.project_path,
                    config_paths,
                    config,
                    worktrees,
                },
            );
        }
        state.projects = watched_projects;
        if watched == state.watched {
            return;
        }

        // FSEvents restarts its stream for each change, so they're applied together.
        let mut failed = Vec::new();
        let mut changes = state.watcher.paths_mut();
        for (path, mode) in &state.watched {
            if watched.get(path) != Some(mode) {
                // A deleted directory may already be unwatched.
                let _result = changes.remove(path.as_std_path());
            }
        }
        for (path, mode) in &watched {
            if state.watched.get(path) != Some(mode)
                && let Err(error) = changes.add(path.as_std_path(), *mode)
            {
                structured_log::file_watch_failed(&self.paths, path.as_str(), &error.to_string());
                failed.push(path.clone());
            }
        }
        if let Err(error) = changes.commit() {
            structured_log::file_watch_failed(&self.paths, "watch list", &error.to_string());
        }
        // The next reload tries these again.
        for path in failed {
            watched.remove(&path);
        }
        state.watched = watched;
        let _result = state.messages.send(WatchMessage::Restarted);
    }

    fn scopes(&self, message: WatchMessage) -> BTreeSet<ReconciliationScope> {
        let mut state = lock_watch_state(&self.state);
        match message {
            WatchMessage::Event(Ok(event)) if !event.need_rescan() => state.event_scopes(&event),
            WatchMessage::Event(Err(error)) => {
                structured_log::file_watch_failed(&self.paths, "file events", &error.to_string());
                state.changes()
            }
            // Events may have been missed, so everything watched is checked.
            WatchMessage::Event(Ok(_)) | WatchMessage::Restarted => state.changes(),
        }
    }
}

impl WatchState {
    fn changes(&mut self) -> BTreeSet<ReconciliationScope> {
        let mut scopes = BTreeSet::new();
        for (project_id, project) in &mut self.projects {
            let config = config_modified_times(&project.path);
            if config != project.config {
                project.config = config;
                scopes.extend(ReconciliationScope::project(project_id.clone()).ok());
            }
            if let Some((git, entries)) = &mut project.worktrees
                && worktree_entries_changed(git, entries)
            {
                scopes.insert(ReconciliationScope::System);
            }
        }

        scopes
    }

    fn event_scopes(&mut self, event: &Event) -> BTreeSet<ReconciliationScope> {
        let mut scopes = BTreeSet::new();
        // Reconciliation reads the config, and reading doesn't change it.
        if matches!(event.kind, EventKind::Access(_)) {
            return scopes;
        }

        for path in &event.paths {
            let Some(path) = Utf8Path::from_path(path) else {
                continue;
            };
            for (project_id, project) in &mut self.projects {
                if project.config_paths.contains(path) {
                    // Every save counts, even one the modified time can't tell apart.
                    project.config = config_modified_times(&project.path);
                    scopes.extend(ReconciliationScope::project(project_id.clone()).ok());
                }
                if let Some((git, entries)) = &mut project.worktrees
                    && path.starts_with(git.join("worktrees"))
                    && worktree_entries_changed(git, entries)
                {
                    scopes.insert(ReconciliationScope::System);
                }
            }
        }

        scopes
    }
}

/// Rereads `git`'s worktree entries into `entries` and returns whether they changed.
fn worktree_entries_changed(git: &Utf8Path, entries: &mut WorktreeEntries) -> bool {
    let current = worktree_entries(git);
    if current == *entries {
        return false;
    }
    *entries = current;

    true
}

/// Reads [`WorktreeEntries`]. Git deletes `worktrees/` with its last worktree, and an unreadable
/// directory reads as empty too; discovery reports why it can't read it.
fn worktree_entries(git: &Utf8Path) -> WorktreeEntries {
    let Ok(entries) = fs::read_dir_paths(&git.join("worktrees")) else {
        return WorktreeEntries::new();
    };

    entries
        .into_iter()
        .map(|entry| {
            let times = [entry.join("gitdir"), entry.join("locked")]
                .map(|path| fs::modified_at(&path).ok().flatten());
            (entry, times)
        })
        .collect()
}

fn load_project_config_watches(
    paths: &PvPaths,
) -> Result<Option<Vec<ProjectConfigWatch>>, DaemonError> {
    let Some(database) = Database::open_read_only(paths)? else {
        return Ok(None);
    };

    Ok(Some(database.project_config_watches()?))
}

/// The canonical directory at `path`. A missing Project is already reported as missing, and is
/// watched again on the first reload after it's back.
fn existing_directory(paths: &PvPaths, path: &Utf8Path) -> Option<Utf8PathBuf> {
    let directory = match fs::canonicalize(path) {
        Ok(directory) => directory,
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return None;
        }
        Err(error) => {
            structured_log::file_watch_failed(paths, path.as_str(), &error.to_string());
            return None;
        }
    };

    matches!(fs::path_is_directory(&directory), Ok(true)).then_some(directory)
}

/// The `.git` directory of the main checkout holding `directory`, which makes a Project linked
/// there a discovery source.
fn repository_git_dir(paths: &PvPaths, directory: &Utf8Path) -> Option<Utf8PathBuf> {
    match crate::worktrees::main_checkout(directory) {
        Ok(main_checkout) => main_checkout.map(|(main_checkout, _path)| main_checkout.join(".git")),
        Err(error) => {
            structured_log::file_watch_failed(paths, directory.as_str(), &error.to_string());
            None
        }
    }
}

/// An unreadable config counts as missing; reconciliation reports why it can't be read.
fn config_modified_times(project_path: &Utf8Path) -> ConfigModifiedTimes {
    ConfigModifiedTimes {
        preferred: fs::modified_at(&project_path.join(PREFERRED_CONFIG_FILE))
            .ok()
            .flatten(),
        alternate: fs::modified_at(&project_path.join(ALTERNATE_CONFIG_FILE))
            .ok()
            .flatten(),
    }
}

fn lock_watch_state(state: &Mutex<WatchState>) -> MutexGuard<'_, WatchState> {
    match state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use camino::Utf8PathBuf;
    use camino_tempfile::{Utf8TempDir, tempdir};
    use notify::Event;
    use notify::event::{AccessKind, EventKind, Flag};
    use state::{Database, LinkProjectInput, PvPaths};
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    use super::{ProjectConfigWatcher, WatchMessage};
    use crate::reconciliation::ReconciliationDebouncer;

    /// Long enough for FSEvents to deliver an event on a busy CI runner.
    const EVENT_TIMEOUT: Duration = Duration::from_secs(5);
    /// How long nothing must arrive to show that a change requested no reconciliation.
    const QUIET_PERIOD: Duration = Duration::from_millis(500);

    struct Fixture {
        _tempdir: Utf8TempDir,
        root: Utf8PathBuf,
        paths: PvPaths,
    }

    impl Fixture {
        fn new() -> Result<Self> {
            let tempdir = tempdir()?;
            // macOS temporary directories sit behind a symlink.
            let root = state::fs::canonicalize(tempdir.path())?;
            let paths = PvPaths::for_home(root.join("home"));

            Ok(Self {
                _tempdir: tempdir,
                root,
                paths,
            })
        }

        /// Links a Project at `name` and returns its directory and reconciliation scope.
        fn link(&self, name: &str) -> Result<(Utf8PathBuf, String)> {
            let path = self.root.join(name);
            let project = Database::open(&self.paths)?
                .link_project(LinkProjectInput {
                    original_path: path.clone(),
                    primary_hostname: format!("{}.test", name.replace('/', "-")),
                    config_path: path.join("pv.yml"),
                    desired_php_track: None,
                    path: path.clone(),
                })?
                .project;

            Ok((path, format!("project:{}", project.id)))
        }

        fn watch(&self) -> Result<(ProjectConfigWatcher, UnboundedReceiver<WatchMessage>)> {
            Ok(ProjectConfigWatcher::start(
                self.paths.clone(),
                &Database::open(&self.paths)?,
            )?)
        }

        /// Watches the linked Projects and records the scopes the watcher requests.
        fn run(&self) -> Result<RunningWatcher> {
            let (watcher, messages) = self.watch()?;
            let (sender, scopes) = unbounded_channel();
            let debouncer = ReconciliationDebouncer::new(Duration::from_millis(50), move |scope| {
                let _result = sender.send(scope.to_string());
            });
            let task = tokio::spawn(watcher.clone().run(messages, debouncer));

            Ok(RunningWatcher {
                watcher,
                scopes,
                task,
            })
        }
    }

    struct RunningWatcher {
        watcher: ProjectConfigWatcher,
        scopes: UnboundedReceiver<String>,
        task: JoinHandle<Result<(), crate::DaemonError>>,
    }

    impl RunningWatcher {
        async fn next_scope(&mut self) -> Result<String> {
            timeout(EVENT_TIMEOUT, self.scopes.recv())
                .await?
                .ok_or_else(|| anyhow!("watcher scope recorder closed"))
        }

        /// The scopes requested, waiting up to [`EVENT_TIMEOUT`] for the first one and then until
        /// nothing arrives for [`QUIET_PERIOD`].
        async fn requested(&mut self) -> Result<Vec<String>> {
            let mut scopes = vec![self.next_scope().await?];
            scopes.extend(self.settle().await);

            Ok(scopes)
        }

        /// The scopes requested until nothing arrives for [`QUIET_PERIOD`].
        async fn settle(&mut self) -> Vec<String> {
            let mut scopes = Vec::new();
            while let Ok(Some(scope)) = timeout(QUIET_PERIOD, self.scopes.recv()).await {
                scopes.push(scope);
            }

            scopes
        }
    }

    impl Drop for RunningWatcher {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn write(path: &camino::Utf8Path, content: &str) -> Result<()> {
        Ok(state::fs::write_sensitive_file(path, content)?)
    }

    #[tokio::test]
    async fn watcher_enqueues_when_preferred_config_appears_after_empty_link() -> Result<()> {
        let fixture = Fixture::new()?;
        let (project_path, scope) = fixture.link("project")?;
        state::fs::ensure_user_dir(&project_path)?;
        let mut watcher = fixture.run()?;

        write(&project_path.join("pv.yml"), "php: '8.4'\n")?;

        assert_eq!(watcher.next_scope().await?, scope);

        Ok(())
    }

    #[tokio::test]
    async fn watcher_enqueues_when_alternate_config_appears_after_empty_link() -> Result<()> {
        let fixture = Fixture::new()?;
        let (project_path, scope) = fixture.link("project")?;
        state::fs::ensure_user_dir(&project_path)?;
        let mut watcher = fixture.run()?;

        write(&project_path.join("pv.yaml"), "php: '8.4'\n")?;

        assert_eq!(watcher.next_scope().await?, scope);

        Ok(())
    }

    #[tokio::test]
    async fn watcher_enqueues_when_project_config_conflict_appears() -> Result<()> {
        let fixture = Fixture::new()?;
        let (project_path, scope) = fixture.link("project")?;
        write(&project_path.join("pv.yml"), "php: '8.3'\n")?;
        let mut watcher = fixture.run()?;

        write(&project_path.join("pv.yaml"), "php: '8.4'\n")?;

        assert_eq!(watcher.next_scope().await?, scope);

        Ok(())
    }

    #[tokio::test]
    async fn watcher_enqueues_a_config_edited_in_place() -> Result<()> {
        let fixture = Fixture::new()?;
        let (project_path, scope) = fixture.link("project")?;
        let config_path = project_path.join("pv.yml");
        write(&config_path, "php: '8.3'\n")?;
        let mut watcher = fixture.run()?;

        state::fs::open_append_file(&config_path)?.write_all(b"serve: true\n")?;

        assert_eq!(watcher.next_scope().await?, scope);

        Ok(())
    }

    #[tokio::test]
    async fn watcher_enqueues_an_edit_to_the_file_a_config_links_to() -> Result<()> {
        let fixture = Fixture::new()?;
        let (project_path, scope) = fixture.link("project")?;
        let target = project_path.join("config/project.yml");
        write(&target, "php: '8.3'\n")?;
        state::fs::symlink_file(&target, &project_path.join("pv.yml"))?;
        let mut watcher = fixture.run()?;

        write(&target, "php: '8.4'\n")?;

        assert_eq!(watcher.requested().await?, vec![scope]);

        Ok(())
    }

    #[tokio::test]
    async fn atomic_save_enqueues_one_reconciliation_and_ignores_other_files() -> Result<()> {
        let fixture = Fixture::new()?;
        let (project_path, scope) = fixture.link("project")?;
        write(&project_path.join("pv.yml"), "php: '8.3'\n")?;
        let mut watcher = fixture.run()?;

        write(&project_path.join(".env"), "APP_NAME=unwatched\n")?;
        assert_eq!(watcher.settle().await, Vec::<String>::new());
        // Editors save through a temporary file renamed over the config.
        write(&project_path.join(".pv.yml.swp"), "php: '8.4'\n")?;
        state::fs::rename(
            &project_path.join(".pv.yml.swp"),
            &project_path.join("pv.yml"),
        )?;

        assert_eq!(watcher.requested().await?, vec![scope]);

        Ok(())
    }

    #[tokio::test]
    async fn reload_follows_linked_and_unlinked_projects() -> Result<()> {
        let fixture = Fixture::new()?;
        let mut watcher = fixture.run()?;
        let (project_path, scope) = fixture.link("project")?;
        state::fs::ensure_user_dir(&project_path)?;

        watcher.watcher.reload();
        write(&project_path.join("pv.yml"), "php: '8.4'\n")?;
        assert_eq!(watcher.requested().await?, vec![scope.clone()]);

        let project_id = scope.trim_start_matches("project:");
        Database::open(&fixture.paths)?.unlink_project(project_id)?;
        watcher.watcher.reload();
        write(&project_path.join("pv.yml"), "php: '8.3'\n")?;

        assert_eq!(watcher.settle().await, Vec::<String>::new());

        Ok(())
    }

    #[tokio::test]
    async fn reload_watches_a_project_directory_that_appeared() -> Result<()> {
        let fixture = Fixture::new()?;
        let (project_path, scope) = fixture.link("project")?;
        let mut watcher = fixture.run()?;

        write(&project_path.join("pv.yml"), "php: '8.4'\n")?;
        assert_eq!(watcher.settle().await, Vec::<String>::new());
        // The check after the watch list restarts finds the config written before.
        watcher.watcher.reload();
        assert_eq!(watcher.next_scope().await?, scope);
        write(&project_path.join("pv.yml"), "php: '8.3'\n")?;

        assert_eq!(watcher.requested().await?, vec![scope]);

        Ok(())
    }

    #[tokio::test]
    async fn rescan_requests_changed_configs_and_worktree_discovery() -> Result<()> {
        let fixture = Fixture::new()?;
        let (project_path, scope) = fixture.link("repo")?;
        state::fs::ensure_user_dir(&project_path.join(".git"))?;
        let (watcher, _messages) = fixture.watch()?;
        let rescan =
            || WatchMessage::Event(Ok(Event::new(EventKind::Other).set_flag(Flag::Rescan)));

        write(&project_path.join("pv.yml"), "php: '8.4'\n")?;
        write(
            &project_path.join(".git/worktrees/feature/gitdir"),
            "/elsewhere/feature/.git\n",
        )?;
        let changed = watcher
            .scopes(rescan())
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let unchanged = watcher
            .scopes(WatchMessage::Restarted)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        assert_eq!(changed, vec!["system".to_string(), scope]);
        assert_eq!(unchanged, Vec::<String>::new());

        Ok(())
    }

    #[tokio::test]
    async fn reading_a_config_requests_nothing() -> Result<()> {
        let fixture = Fixture::new()?;
        let (project_path, _scope) = fixture.link("project")?;
        write(&project_path.join("pv.yml"), "php: '8.4'\n")?;
        let (watcher, _messages) = fixture.watch()?;
        // inotify reports opening a file, and reconciliation opens the config to read it.
        let open = Event::new(EventKind::Access(AccessKind::Any))
            .add_path(project_path.join("pv.yml").into_std_path_buf());

        assert!(watcher.scopes(WatchMessage::Event(Ok(open))).is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn worktree_entry_changes_request_system_reconciliation() -> Result<()> {
        let fixture = Fixture::new()?;
        let (main_checkout, _scope) = fixture.link("repo")?;
        let git = main_checkout.join(".git");
        state::fs::ensure_user_dir(&git)?;
        let mut watcher = fixture.run()?;
        let entry = git.join("worktrees/feature");
        let system = vec!["system".to_string()];

        // The first worktree creates `.git/worktrees/`, which the system job's reload watches.
        write(&entry.join("gitdir"), "/elsewhere/feature/.git\n")?;
        write(&entry.join("locked"), "initializing\n")?;
        assert_eq!(watcher.requested().await?, system);
        watcher.watcher.reload();
        assert_eq!(watcher.settle().await, Vec::<String>::new());
        // Every Git command run in the worktree rewrites its index.
        write(&entry.join("index"), "index\n")?;
        assert_eq!(watcher.settle().await, Vec::<String>::new());
        // `git worktree add` has checked the worktree out.
        state::fs::remove_file(&entry.join("locked"))?;
        assert_eq!(watcher.requested().await?, system);
        // `git worktree move`.
        write(&entry.join("gitdir"), "/moved/feature/.git\n")?;
        assert_eq!(watcher.requested().await?, system);
        // `git worktree remove`.
        state::fs::delete_dir_all(&entry)?;

        assert_eq!(watcher.requested().await?, system);

        Ok(())
    }

    #[tokio::test]
    async fn reload_requests_discovery_for_newly_linked_sources_with_worktrees() -> Result<()> {
        let fixture = Fixture::new()?;
        let worktree_entry = ".git/worktrees/feature/gitdir";
        let (startup_source, _scope) = fixture.link("startup")?;
        write(&startup_source.join(worktree_entry), "/elsewhere/a/.git\n")?;
        let mut watcher = fixture.run()?;
        // Startup reconciliation discovers the worktrees that already exist.
        assert_eq!(watcher.settle().await, Vec::<String>::new());

        let (linked_source, _scope) = fixture.link("linked")?;
        write(&linked_source.join(worktree_entry), "/elsewhere/b/.git\n")?;
        watcher.watcher.reload();
        assert_eq!(watcher.requested().await?, vec!["system".to_string()]);
        // A second source in a repository another source already watches, such as another app
        // in a monorepo.
        let (second_source, _scope) = fixture.link("startup/apps/api")?;
        state::fs::ensure_user_dir(&second_source)?;
        watcher.watcher.reload();

        assert_eq!(watcher.requested().await?, vec!["system".to_string()]);

        Ok(())
    }
}
