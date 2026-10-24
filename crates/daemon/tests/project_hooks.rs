#![cfg(unix)]

use std::fs::symlink_metadata;
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use insta::assert_debug_snapshot;
use pv_fake::{EventKind, FakeSettings, InstalledFake, Pause, Persona};
use rusqlite::Connection;
use rustix::io::Errno;
use rustix::process::{Pid, test_kill_process};
use state::{
    Database, GatewayPort, JobRecord, JobStatus, LinkProjectInput, PortRequest, ProjectRecord,
    PvPaths,
};
use tokio::time::{sleep, timeout};

struct Fixture {
    _tempdir: Utf8TempDir,
    root: Utf8PathBuf,
    paths: PvPaths,
    source: ProjectRecord,
    _caddy: InstalledFake,
}

impl Fixture {
    fn new() -> Result<Self> {
        let tempdir = tempdir()?;
        let root = state::fs::canonicalize(tempdir.path())?;
        let paths = PvPaths::for_home(root.join("home"));
        let path = root.join("source");
        state::fs::ensure_user_dir(&path.join(".git"))?;
        state::fs::write_sensitive_file(&path.join("pv.yml"), "serve: false\n")?;
        state::fs::write_sensitive_file(&path.join(".env"), "SOURCE_ENV=copied\n")?;
        let source = Database::open(&paths)?
            .link_project(LinkProjectInput {
                path: path.clone(),
                original_path: path.clone(),
                primary_hostname: "source.test".to_owned(),
                config_path: path.join("pv.yml"),
                desired_php_track: None,
            })?
            .project;
        pv_fake::seed_dns_port(&paths)?;
        pv_fake::install_monitor(&paths)?;
        daemon::gateway::use_unrouted_public_probe_ports_for_test();
        let release = paths.home().join("caddy-release");
        let caddy = pv_fake::install(&release.join("bin/caddy"), Persona::Caddy)?;
        let ca = platform::generate_local_ca()?;
        state::fs::write_sensitive_file(&paths.ca_certificate(), &ca.certificate_pem)?;
        state::fs::write_sensitive_file(&paths.ca_private_key(), &ca.private_key_pem)?;
        let mut database = Database::open(&paths)?;
        database.record_managed_resource_track_installed(
            "caddy",
            "2",
            "fake-caddy-pv1",
            &release,
        )?;
        for kind in [GatewayPort::Http, GatewayPort::Https] {
            let port = pv_fake::port()?;
            database.assign_port(PortRequest::gateway(kind, port, port, port), |_port| true)?;
        }
        Ok(Self {
            _tempdir: tempdir,
            root,
            paths,
            source,
            _caddy: caddy,
        })
    }

    fn worktree(
        &self,
        name: &str,
        prepare: Option<&str>,
        setup: Option<&str>,
    ) -> Result<Utf8PathBuf> {
        let path = self.root.join(name);
        let entry = self.source.path.join(".git/worktrees").join(name);
        state::fs::write_sensitive_file(&entry.join("gitdir"), &format!("{path}/.git\n"))?;
        state::fs::write_sensitive_file(&entry.join("commondir"), "../..\n")?;
        state::fs::write_sensitive_file(&path.join(".git"), &format!("gitdir: {entry}\n"))?;
        let mut config = "serve: false\nenv:\n  READY: ready\nhooks:\n".to_owned();
        for (phase, command) in [("prepare", prepare), ("setup", setup)] {
            if let Some(command) = command {
                config.push_str(&format!(
                    "  {phase}:\n    - {}\n",
                    serde_json::to_string(command)?
                ));
            }
        }
        state::fs::write_sensitive_file(&path.join("pv.yml"), &config)?;
        // The user approved these commands once, as `pv link` in a terminal would.
        let commands = [prepare, setup]
            .into_iter()
            .flatten()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        Database::open(&self.paths)?
            .approve_hook_commands(&daemon::hook_approval_folder(&path), &commands)?;
        Ok(path)
    }

    async fn start(&self) -> Result<daemon::RunningDaemon> {
        Ok(
            daemon::RunningDaemon::start_without_managed_resource_adapters(self.paths.clone())
                .await?,
        )
    }

    fn hook_jobs(&self) -> Result<Vec<JobRecord>> {
        Ok(Database::open(&self.paths)?
            .recent_jobs()?
            .into_iter()
            .filter(|job| job.kind == "hooks")
            .collect())
    }
}

// Hook commands are shell behavior, not a runtime fake. Every held command gives up after 150s.
fn hold_until(path: &str) -> String {
    format!(
        "attempt=0; while [ ! -f {path} ]; do [ \"$attempt\" -lt 1500 ] || exit 91; attempt=$((attempt + 1)); sleep 0.1; done"
    )
}

async fn eventually<T>(mut inspect: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    timeout(Duration::from_secs(20), async {
        loop {
            if let Some(value) = inspect()? {
                return Ok(value);
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_elapsed| anyhow!("hook fixture timed out"))?
}

async fn file_exists(path: &Utf8Path) -> Result<()> {
    eventually(|| Ok(state::fs::path_entry_exists(path)?.then_some(()))).await
}

async fn reconcile(paths: &PvPaths, scope: &str) -> Result<()> {
    let job = daemon::submit_job(paths.clone(), "reconcile", scope).await?;
    let completed = eventually(|| {
        Ok(Database::open(paths)?
            .job(&job.id)?
            .filter(|job| job.status != JobStatus::Running))
    })
    .await?;
    anyhow::ensure!(
        completed.status == JobStatus::Succeeded,
        "reconciliation failed: {:?}",
        completed.error
    );
    Ok(())
}

async fn shutdown_after<T>(
    daemon: daemon::RunningDaemon,
    paths: &PvPaths,
    operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    let outcome = operation.await;
    let shutdown = daemon.shutdown().await;
    let cleanup = daemon::stop_recorded_runtimes(paths.clone()).await;
    let cleanup = match (shutdown, cleanup) {
        (Ok(()), cleanup) => cleanup,
        (Err(error), Ok(())) => Err(error),
        (Err(original), Err(cleanup)) => {
            return Err(anyhow!(
                "{original}; runtime cleanup also failed: {cleanup}; original test outcome: {outcome:?}",
                outcome = outcome.as_ref().err()
            ));
        }
    };
    match (outcome, cleanup) {
        (outcome, Ok(())) => outcome,
        (Ok(_), Err(error)) => Err(error.into()),
        (Err(original), Err(cleanup)) => Err(anyhow!(
            "{original:#}; daemon shutdown also failed: {cleanup}"
        )),
    }
}

#[tokio::test]
async fn setup_runs_after_own_apply_when_an_unrelated_project_fails() -> Result<()> {
    let fixture = Fixture::new()?;
    let path = fixture.worktree(
        "ready",
        None,
        Some("grep -q '^READY=ready$' .env && touch setup-ran"),
    )?;
    // Pre-create the env file so the source's invalid config cannot block discovery during env copying.
    state::fs::write_sensitive_file(&path.join(".env"), "")?;
    state::fs::write_sensitive_file(&fixture.source.config_path, "unknown: invalid\n")?;
    let daemon = fixture.start().await?;
    shutdown_after(daemon, &fixture.paths, async {
        file_exists(&path.join("setup-ran")).await?;
        let job = eventually(|| {
            Ok(Database::open(&fixture.paths)?
                .job("job_000001")?
                .filter(|job| job.status != JobStatus::Running))
        })
        .await?;
        assert_eq!(job.status, JobStatus::Failed);
        let project = Database::open(&fixture.paths)?
            .project_by_path(&path)?
            .ok_or_else(|| anyhow!("ready Project missing"))?;
        assert!(
            !Database::open(&fixture.paths)?
                .held_project_ids()?
                .contains(&project.id)
        );
        eventually(|| {
            Ok(fixture
                .hook_jobs()?
                .iter()
                .all(|job| job.status == JobStatus::Succeeded)
                .then_some(()))
        })
        .await?;
        assert_eq!(fixture.hook_jobs()?.len(), 1);
        Ok(())
    })
    .await
}

#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "permission assertions inspect the log and directory metadata without following symlinks"
)]
async fn hooks_run_four_at_a_time_in_fifo_order_and_prepared_worktrees_link_once() -> Result<()> {
    let fixture = Fixture::new()?;
    let prepare = format!(
        "test -f .env && printf 'prepare\\n' >> prepare-count && touch started && {}",
        hold_until("release")
    );
    let setup = "grep -q '^READY=ready$' .env && test -f \"$PV_SOURCE_PATH/.env\" && printf 'setup\\n' > setup-done";
    let worktrees = (0..6)
        .map(|index| fixture.worktree(&format!("tree-{index}"), Some(&prepare), Some(setup)))
        .collect::<Result<Vec<_>>>()?;
    let daemon = fixture.start().await?;
    shutdown_after(daemon, &fixture.paths, async {
        eventually(|| Ok((fixture.hook_jobs()?.len() == 6).then_some(()))).await?;
        eventually(|| {
            Ok((worktrees
                .iter()
                .filter(|path| path.join("started").exists())
                .count()
                == 4)
                .then_some(()))
        })
        .await?;
        let mut jobs = fixture.hook_jobs()?;
        jobs.sort_by(|first, second| first.id.cmp(&second.id));
        let ordered = jobs
            .iter()
            .map(|job| {
                let worktree_id = job
                    .scope
                    .rsplit(':')
                    .next()
                    .ok_or_else(|| anyhow!("missing worktree ID"))?;
                worktrees
                    .iter()
                    .find(|path| {
                        let name = path.file_name().unwrap_or_default();
                        state::fs::read_to_string(
                            &fixture
                                .source
                                .path
                                .join(".git/worktrees")
                                .join(name)
                                .join("pv-id"),
                        )
                        .is_ok_and(|value| value.trim() == worktree_id)
                    })
                    .cloned()
                    .ok_or_else(|| anyhow!("missing worktree for {}", job.scope))
            })
            .collect::<Result<Vec<_>>>()?;
        assert!(
            ordered[..4]
                .iter()
                .all(|path| path.join("started").exists())
        );
        assert!(!ordered[4].join("started").exists());
        assert!(!ordered[5].join("started").exists());
        // An occupied hook pool does not hold jobs.lock or delay an unrelated reconciliation.
        timeout(
            Duration::from_secs(10),
            reconcile(&fixture.paths, &format!("project:{}", fixture.source.id)),
        )
        .await??;
        state::fs::write_sensitive_file(&ordered[0].join("release"), "")?;
        file_exists(&ordered[4].join("started")).await?;
        assert!(!ordered[5].join("started").exists());
        for path in &worktrees {
            state::fs::write_sensitive_file(&path.join("release"), "")?;
        }
        for path in &worktrees {
            file_exists(&path.join("setup-done")).await?;
        }
        eventually(|| {
            Ok(fixture
                .hook_jobs()?
                .iter()
                .all(|job| job.status == JobStatus::Succeeded)
                .then_some(()))
        })
        .await?;
        let database = Database::open(&fixture.paths)?;
        let discovered = database.discovered_projects()?;
        let runs = worktrees
            .iter()
            .map(|path| {
                Ok((
                    state::fs::read_to_string(&path.join("prepare-count"))?,
                    state::fs::read_to_string(&path.join("setup-done"))?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let log = fixture.paths.project_hook_log(&discovered[0].project_id);
        assert_eq!(symlink_metadata(&log)?.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            symlink_metadata(log.parent().ok_or_else(|| anyhow!("log parent missing"))?)?
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_debug_snapshot!((discovered.len(), fixture.hook_jobs()?.len(), runs));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn unapproved_hooks_are_skipped_and_the_worktree_is_still_provisioned() -> Result<()> {
    let fixture = Fixture::new()?;
    let path = fixture.worktree("changed", Some("touch prepared"), Some("touch set-up"))?;
    // The branch changed a command after the user approved the originals.
    let config = state::fs::read_to_string(&path.join("pv.yml"))?
        .replace("touch set-up", "touch changed-set-up");
    state::fs::write_sensitive_file(&path.join("pv.yml"), &config)?;
    let daemon = fixture.start().await?;
    shutdown_after(daemon, &fixture.paths, async {
        let skipped = eventually(|| {
            let jobs = fixture.hook_jobs()?;
            Ok(
                (jobs.len() == 2 && jobs.iter().all(|job| job.status == JobStatus::Succeeded))
                    .then_some(jobs),
            )
        })
        .await?;
        // Linking didn't wait, and neither list ran.
        assert_eq!(
            Database::open(&fixture.paths)?.discovered_projects()?.len(),
            1
        );
        assert!(!path.join("prepared").exists());
        assert!(!path.join("changed-set-up").exists());
        let mut phases = skipped
            .iter()
            .filter_map(|job| job.summary.as_deref())
            .filter_map(|summary| summary.split_once(" skipped: hook commands aren't approved"))
            .map(|(phase, _)| phase)
            .collect::<Vec<_>>();
        phases.sort_unstable();
        assert_eq!(phases, ["prepare", "setup"]);
        assert!(
            Database::open(&fixture.paths)?
                .unresolved_job_failures()?
                .iter()
                .all(|failure| failure.job.kind != "hooks")
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn failed_prepare_waits_for_restart_and_a_success_clears_its_failure() -> Result<()> {
    let fixture = Fixture::new()?;
    let path = fixture.worktree(
        "failed",
        Some("printf 'attempt\\n' >> attempts; test -f \"$PV_SOURCE_PATH/fixed\""),
        Some("touch setup-done"),
    )?;
    let daemon = fixture.start().await?;
    let outcome = async {
        eventually(|| {
            Ok(fixture
                .hook_jobs()?
                .into_iter()
                .find(|job| job.status == JobStatus::Failed))
        })
        .await?;
        reconcile(&fixture.paths, "system").await?;
        reconcile(&fixture.paths, "system").await?;
        assert_eq!(fixture.hook_jobs()?.len(), 1);
        assert!(
            Database::open(&fixture.paths)?
                .discovered_projects()?
                .is_empty()
        );
        state::fs::write_sensitive_file(&fixture.source.path.join("fixed"), "")?;
        Ok(())
    };
    shutdown_after(daemon, &fixture.paths, outcome).await?;
    let daemon = fixture.start().await?;
    shutdown_after(daemon, &fixture.paths, async {
        file_exists(&path.join("setup-done")).await?;
        eventually(|| {
            Ok(fixture
                .hook_jobs()?
                .iter()
                .all(|job| job.status != JobStatus::Running)
                .then_some(()))
        })
        .await?;
        let failures = Database::open(&fixture.paths)?
            .unresolved_job_failures()?
            .into_iter()
            .filter(|failure| failure.job.kind == "hooks")
            .count();
        assert_debug_snapshot!((
            state::fs::read_to_string(&path.join("attempts"))?,
            fixture.hook_jobs()?.len(),
            failures
        ));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn queued_setup_is_recorded_and_interrupted_startup_keeps_its_log_path() -> Result<()> {
    let fixture = Fixture::new()?;
    let setup = format!("touch started; {}", hold_until("release"));
    let paths = (0..6)
        .map(|index| fixture.worktree(&format!("setup-{index}"), None, Some(&setup)))
        .collect::<Result<Vec<_>>>()?;
    let daemon = fixture.start().await?;
    shutdown_after(daemon, &fixture.paths, async {
        eventually(|| Ok((fixture.hook_jobs()?.len() == 6).then_some(()))).await?;
        eventually(|| {
            Ok((paths
                .iter()
                .filter(|path| path.join("started").exists())
                .count()
                == 4)
                .then_some(()))
        })
        .await?;
        assert!(fixture.hook_jobs()?.iter().all(|job| {
            job.status == JobStatus::Running
                && job
                    .summary
                    .as_deref()
                    .is_some_and(|summary| summary.contains("log:"))
        }));
        Ok(())
    })
    .await?;
    assert!(
        fixture
            .hook_jobs()?
            .iter()
            .all(|job| job.status == JobStatus::Failed)
    );
    let project = Database::open(&fixture.paths)?
        .discovered_projects()?
        .remove(0);
    let scope = format!("project:{}", project.project_id);
    let log = fixture.paths.project_hook_log(&project.project_id);
    let lost = Database::open(&fixture.paths)?.start_job_with_summary(
        "hooks",
        &scope,
        Some(&format!("setup hooks queued (log: {log})")),
    )?;
    let daemon = fixture.start().await?;
    shutdown_after(daemon, &fixture.paths, async {
        let job = Database::open(&fixture.paths)?
            .job(&lost.id)?
            .ok_or_else(|| anyhow!("lost hook job missing"))?;
        assert_eq!(job.status, JobStatus::Failed);
        assert!(
            job.error
                .as_deref()
                .is_some_and(|error| error.contains(log.as_str()))
        );
        reconcile(&fixture.paths, "system").await?;
        assert_eq!(fixture.hook_jobs()?.len(), 7);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn removed_worktree_stops_its_hook_before_removing_the_project_and_logs() -> Result<()> {
    let fixture = Fixture::new()?;
    let setup = format!(
        "printf '%s\\n' \"$$\" > pid.tmp; mv pid.tmp pid; touch started; {}",
        hold_until("release")
    );
    let path = fixture.worktree("removed", Some("printf prepare"), Some(&setup))?;
    let daemon = fixture.start().await?;
    shutdown_after(daemon, &fixture.paths, async {
        file_exists(&path.join("started")).await?;
        let process = state::fs::read_to_string(&path.join("pid"))?
            .trim()
            .parse::<i32>()?;
        let process = Pid::from_raw(process).ok_or_else(|| anyhow!("invalid hook PID"))?;
        let project = Database::open(&fixture.paths)?
            .discovered_projects()?
            .remove(0);
        let setup_job = fixture
            .hook_jobs()?
            .into_iter()
            .find(|job| job.scope == format!("project:{}", project.project_id))
            .ok_or_else(|| anyhow!("setup job missing"))?;
        state::fs::delete_dir_all(&fixture.source.path.join(".git/worktrees/removed"))?;
        eventually(|| {
            Ok(Database::open(&fixture.paths)?
                .project_by_id(&project.project_id)?
                .is_none()
                .then_some(()))
        })
        .await?;
        assert_eq!(test_kill_process(process), Err(Errno::SRCH));
        let job = Database::open(&fixture.paths)?
            .job(&setup_job.id)?
            .ok_or_else(|| anyhow!("setup job missing"))?;
        assert_eq!(job.status, JobStatus::Failed);
        assert!(!fixture.paths.project_hook_log(&project.project_id).exists());
        assert!(
            !fixture
                .paths
                .worktree_hook_log(&fixture.source.id, &project.worktree_id)
                .exists()
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn requested_first_reconciliation_runs_daemon_setup_once() -> Result<()> {
    let fixture = Fixture::new()?;
    let path = fixture.worktree("terminal", None, Some("printf setup >> setup-count"))?;
    state::fs::write_sensitive_file(
        &fixture.source.path.join(".git/worktrees/terminal/pv-id"),
        "terminal\n",
    )?;
    let input = state::LinkDiscoveredProjectInput {
        source_project_id: fixture.source.id.clone(),
        worktree_id: "terminal".to_owned(),
        worktree_name: "terminal".to_owned(),
        path: path.clone(),
        config_path: path.join("pv.yml"),
        mode: state::ProjectMode::ResourceOnly,
    };
    // Keep startup from applying this worktree; its first successful apply will follow an explicit request.
    let mut database = Database::open(&fixture.paths)?;
    let initial = database.link_discovered_project(input.clone())?;
    database.unlink_project(&initial.id)?;
    let daemon = fixture.start().await?;
    shutdown_after(daemon, &fixture.paths, async {
        reconcile(&fixture.paths, "system").await?;
        // Insert a discovered Project whose first reconciliation is explicitly requested.
        let project = Database::open(&fixture.paths)?.link_discovered_project(input)?;
        reconcile(&fixture.paths, &format!("project:{}", project.id)).await?;
        reconcile(&fixture.paths, &format!("project:{}", project.id)).await?;
        file_exists(&path.join("setup-count")).await?;
        eventually(|| {
            Ok(fixture
                .hook_jobs()?
                .into_iter()
                .all(|job| job.status == JobStatus::Succeeded)
                .then_some(()))
        })
        .await?;
        assert_eq!(fixture.hook_jobs()?.len(), 1);
        assert_eq!(
            state::fs::read_to_string(&path.join("setup-count"))?,
            "setup"
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn unlinking_a_source_stops_running_and_queued_hooks_for_its_children() -> Result<()> {
    let fixture = Fixture::new()?;
    let setup = format!("touch started; {}", hold_until("release"));
    let paths = (0..6)
        .map(|index| fixture.worktree(&format!("unlink-{index}"), None, Some(&setup)))
        .collect::<Result<Vec<_>>>()?;
    let daemon = fixture.start().await?;
    shutdown_after(daemon, &fixture.paths, async {
        eventually(|| Ok((fixture.hook_jobs()?.len() == 6).then_some(())))
            .await
            .context("queue all six setup hooks")?;
        eventually(|| {
            Ok((paths
                .iter()
                .filter(|path| path.join("started").exists())
                .count()
                == 4)
                .then_some(()))
        })
        .await
        .context("start four setup hooks")?;
        Database::open(&fixture.paths)?.unlink_project(&fixture.source.id)?;
        reconcile(&fixture.paths, "system").await?;
        eventually(|| {
            Ok(fixture
                .hook_jobs()?
                .iter()
                .all(|job| job.status == JobStatus::Failed)
                .then_some(()))
        })
        .await
        .context("cancel the active and queued setup hooks")?;
        assert!(Database::open(&fixture.paths)?.projects()?.is_empty());
        assert!(
            Database::open(&fixture.paths)?
                .unresolved_job_failures()?
                .iter()
                .all(|failure| failure.job.kind != "hooks")
        );
        assert_eq!(
            paths
                .iter()
                .filter(|path| path.join("started").exists())
                .count(),
            4
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn allocation_cleanup_observes_a_stopped_hook_before_dropping_its_database() -> Result<()> {
    let fixture = Fixture::new()?;
    let release = fixture
        .paths
        .resources()
        .join("postgres/18/releases/18.0-pv1");
    pv_fake::install(&release.join("bin/initdb"), Persona::Initdb)?;
    let _postgres = pv_fake::install(&release.join("bin/postgres"), Persona::Postgres)?;
    for file in [
        "share/postgres.bki",
        "lib/libcrypto.3.dylib",
        "lib/libssl.3.dylib",
        "lib/postgresql/pg_trgm.dylib",
        "lib/postgresql/pgcrypto.dylib",
        "lib/postgresql/sslinfo.dylib",
        "share/extension/pg_trgm.control",
        "share/extension/pgcrypto.control",
        "share/extension/sslinfo.control",
    ] {
        state::fs::write_sensitive_file(&release.join(file), "fixture support file")?;
    }
    let mut database = Database::open(&fixture.paths)?;
    database.record_managed_resource_track_installed("postgres", "18", "18.0-pv1", &release)?;
    let port = pv_fake::port()?;
    database.assign_port(
        PortRequest::resource_port("postgres", "18", "postgres", port, port, port),
        platform::loopback_tcp_port_available,
    )?;
    let setup = format!(
        "printf '%s\\n' \"$$\" > pid.tmp; mv pid.tmp pid; touch started; {}",
        hold_until("release-hook")
    );
    let path = fixture.worktree("database", None, Some(&setup))?;
    let config = state::fs::read_to_string(&path.join("pv.yml"))?.replacen(
        "serve: false",
        "serve: true\nroot: .",
        1,
    );
    state::fs::write_sensitive_file(
        &path.join("pv.yml"),
        &format!(
            "{config}postgres:\n  version: '18'\n  allocations:\n    app-db:\n      env:\n        DATABASE_URL: '${{url}}'\n"
        ),
    )?;
    let daemon = daemon::RunningDaemon::start(fixture.paths.clone()).await?;
    shutdown_after(daemon, &fixture.paths, async {
        file_exists(&path.join("started")).await?;
        let project = Database::open(&fixture.paths)?.project_by_path(&path)?.ok_or_else(|| anyhow!("worktree Project missing"))?;
        let allocation = Database::open(&fixture.paths)?.resource_allocations(&project.id, "postgres")?.remove(0);
        let database_file = fixture.paths.resource_data_dir("postgres", "18").join("databases").join(allocation.generated_name);
        assert!(database_file.exists());
        let process = Pid::from_raw(state::fs::read_to_string(&path.join("pid"))?.trim().parse::<i32>()?).ok_or_else(|| anyhow!("invalid hook PID"))?;
        let release_validation = fixture.root.join("release-cleanup-gateway");
        let caddy = pv_fake::install_with_settings(fixture._caddy.executable(), Persona::Caddy, FakeSettings {
            validate_pause: Some(Pause { until: release_validation.clone() }), ..FakeSettings::default()
        })?;
        state::fs::delete_dir_all(&fixture.source.path.join(".git/worktrees/database"))?;
        eventually(|| Ok(caddy.events()?.iter().any(|event| matches!(&event.kind, EventKind::Held { until } if until == &release_validation)).then_some(()))).await?;
        // Gateway validation holds the job before allocation cleanup. The backing database
        // still exists, and its migration's process must already be gone.
        assert!(database_file.exists());
        assert_eq!(test_kill_process(process), Err(Errno::SRCH));
        assert_eq!(Database::open(&fixture.paths)?.pending_cleanups()?.len(), 1);
        state::fs::write_sensitive_file(&release_validation, "")?;
        eventually(|| Ok((!database_file.exists() && Database::open(&fixture.paths)?.pending_cleanups()?.is_empty()).then_some(()))).await?;
        Ok(())
    }).await
}

#[tokio::test]
async fn outcome_recording_failure_keeps_daemon_running_and_startup_recovers_the_row() -> Result<()>
{
    let fixture = Fixture::new()?;
    let failing = fixture.worktree(
        "failing-record",
        Some(&format!(
            "touch started; {}; exit 23",
            hold_until("release")
        )),
        None,
    )?;
    let held = fixture.worktree(
        "held-record",
        Some(&format!("touch started; {}", hold_until("release"))),
        None,
    )?;
    let daemon = fixture.start().await?;
    let interrupted_id = shutdown_after(daemon, &fixture.paths, async {
        file_exists(&failing.join("started")).await?;
        file_exists(&held.join("started")).await?;
        let worktree_id = state::fs::read_to_string(&fixture.source.path.join(".git/worktrees/failing-record/pv-id"))?;
        let scope = format!("worktree:{}:{}", fixture.source.id, worktree_id.trim());
        let job = Database::open(&fixture.paths)?.active_hook_job(&scope)?.ok_or_else(|| anyhow!("failing job missing"))?;
        Connection::open(fixture.paths.db())?.execute_batch("CREATE TRIGGER reject_hook_outcomes BEFORE UPDATE ON jobs WHEN OLD.kind = 'hooks' AND NEW.status != 'running' BEGIN SELECT RAISE(ABORT, 'hook outcome fixture'); END;")?;
        state::fs::write_sensitive_file(&failing.join("release"), "")?;
        eventually(|| Ok(state::fs::read_to_string(&fixture.paths.daemon_log())?.contains("hook outcome fixture").then_some(()))).await?;
        let log = state::fs::read_to_string(&fixture.paths.daemon_log())?;
        assert!(log.contains("exit status: 23"));
        Connection::open(fixture.paths.db())?.execute_batch("DROP TRIGGER reject_hook_outcomes")?;
        // A fresh reconciliation proves the daemon is still serving after the recording error.
        reconcile(&fixture.paths, "system").await?;
        assert_eq!(Database::open(&fixture.paths)?.job(&job.id)?.map(|job| job.status), Some(JobStatus::Running));
        state::fs::write_sensitive_file(&held.join("release"), "")?;
        eventually(|| Ok(Database::open(&fixture.paths)?.project_by_path(&held)?.map(|_| ()))).await?;
        Ok(job.id)
    }).await?;
    let daemon = fixture.start().await?;
    shutdown_after(daemon, &fixture.paths, async {
        let recovered = Database::open(&fixture.paths)?
            .job(&interrupted_id)?
            .ok_or_else(|| anyhow!("interrupted row missing"))?;
        assert_eq!(recovered.status, JobStatus::Failed);
        reconcile(&fixture.paths, "system").await?;
        Ok(())
    })
    .await
}

#[tokio::test]
async fn removed_and_ordinary_linked_prepares_clear_running_queued_and_failed_scopes() -> Result<()>
{
    for ordinary in [false, true] {
        let fixture = Fixture::new()?;
        let failed = fixture.worktree("failed", Some("exit 17"), None)?;
        let daemon = fixture.start().await?;
        shutdown_after(daemon, &fixture.paths, async {
            let failed_job = eventually(|| {
                Ok(fixture
                    .hook_jobs()?
                    .into_iter()
                    .find(|job| job.status == JobStatus::Failed))
            })
            .await?;
            let mut active = Vec::new();
            for index in 0..4 {
                active.push(fixture.worktree(
                    &format!("active-{index}"),
                    Some(&format!(
                        "touch started; {}; touch unexpected-completion",
                        hold_until("release")
                    )),
                    None,
                )?);
            }
            reconcile(&fixture.paths, "system").await?;
            for path in &active {
                file_exists(&path.join("started")).await?;
            }
            let queued = fixture.worktree("queued", Some("touch unexpected-queued"), None)?;
            reconcile(&fixture.paths, "system").await?;
            let mut scopes = Vec::new();
            let mut database = Database::open(&fixture.paths)?;
            for (name, path) in [
                ("failed", &failed),
                ("active-0", &active[0]),
                ("queued", &queued),
            ] {
                let entry = fixture.source.path.join(".git/worktrees").join(name);
                let worktree_id = state::fs::read_to_string(&entry.join("pv-id"))?;
                scopes.push(format!(
                    "worktree:{}:{}",
                    fixture.source.id,
                    worktree_id.trim()
                ));
                if ordinary {
                    database.link_project_with_mode(
                        LinkProjectInput {
                            path: path.clone(),
                            original_path: path.clone(),
                            primary_hostname: format!("{name}.test"),
                            config_path: path.join("pv.yml"),
                            desired_php_track: None,
                        },
                        state::ProjectMode::ResourceOnly,
                    )?;
                } else {
                    state::fs::delete_dir_all(&entry)?;
                }
            }
            // The fifth prepare remains queued behind four held runs.
            assert!(database.active_hook_job(&scopes[2])?.is_some());
            assert!(!queued.join("unexpected-queued").exists());
            reconcile(&fixture.paths, "system").await?;
            let summary = if ordinary {
                "prepare skipped: linked as an ordinary Project"
            } else {
                "prepare skipped: worktree removed"
            };
            for scope in &scopes {
                let skipped = fixture
                    .hook_jobs()?
                    .into_iter()
                    .find(|job| &job.scope == scope)
                    .ok_or_else(|| anyhow!("skipped job missing"))?;
                assert_eq!(skipped.status, JobStatus::Succeeded);
                assert_eq!(skipped.summary.as_deref(), Some(summary));
                assert!(database.active_hook_job(scope)?.is_none());
            }
            assert_eq!(
                database.job(&failed_job.id)?.map(|job| job.status),
                Some(JobStatus::Failed)
            );
            assert!(database.unresolved_job_failures()?.is_empty());
            for path in &active {
                state::fs::write_sensitive_file(&path.join("release"), "")?;
            }
            for path in &active[1..] {
                eventually(|| Ok(database.project_by_path(path)?.map(|_| ()))).await?;
            }
            reconcile(&fixture.paths, "system").await?;
            assert!(!active[0].join("unexpected-completion").exists());
            assert!(!queued.join("unexpected-queued").exists());
            let discovered = database.discovered_projects()?;
            assert!(
                discovered
                    .iter()
                    .all(|project| ![&failed, &active[0], &queued].contains(&&project.path))
            );
            assert!(database.unresolved_job_failures()?.is_empty());
            Ok(())
        })
        .await?;
    }
    Ok(())
}
