use camino::{Utf8Path, Utf8PathBuf};

use crate::StateError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PvPaths {
    home: Utf8PathBuf,
    root: Utf8PathBuf,
    db: Utf8PathBuf,
    bin: Utf8PathBuf,
    run: Utf8PathBuf,
    logs: Utf8PathBuf,
    downloads: Utf8PathBuf,
    config: Utf8PathBuf,
    certificates: Utf8PathBuf,
    composer: Utf8PathBuf,
    resources: Utf8PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathSummaryEntry {
    pub name: &'static str,
    pub path: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct PathEntry<'path> {
    name: &'static str,
    path: &'path Utf8Path,
    layout_directory: bool,
}

impl PvPaths {
    pub fn for_home(home: impl AsRef<Utf8Path>) -> Self {
        let home = home.as_ref().to_path_buf();
        let root = home.join(".pv");

        Self {
            home,
            db: root.join("pv.db"),
            bin: root.join("bin"),
            run: root.join("run"),
            logs: root.join("logs"),
            downloads: root.join("downloads"),
            config: root.join("config"),
            certificates: root.join("certificates"),
            composer: root.join("composer"),
            resources: root.join("resources"),
            root,
        }
    }

    pub fn default_home() -> Result<Self, StateError> {
        let home = home::home_dir().ok_or(StateError::MissingHome)?;
        let home =
            Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

        Ok(Self::for_home(home))
    }

    pub fn home(&self) -> &Utf8Path {
        &self.home
    }

    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    pub fn db(&self) -> &Utf8Path {
        &self.db
    }

    pub fn bin(&self) -> &Utf8Path {
        &self.bin
    }

    pub fn app_releases_dir(&self) -> Utf8PathBuf {
        self.bin().join("releases")
    }

    pub fn app_release_binary(&self, version: &str) -> Utf8PathBuf {
        self.app_releases_dir().join(version).join("pv")
    }

    pub fn active_pv_binary(&self) -> Utf8PathBuf {
        self.bin().join("pv")
    }

    pub fn run(&self) -> &Utf8Path {
        &self.run
    }

    pub fn update_lock(&self) -> Utf8PathBuf {
        self.run().join("update.lock")
    }

    pub fn daemon_socket(&self) -> Utf8PathBuf {
        self.run().join("pv.sock")
    }

    pub fn daemon_startup_error(&self) -> Utf8PathBuf {
        self.run().join("daemon-startup-error.json")
    }

    pub fn logs(&self) -> &Utf8Path {
        &self.logs
    }

    pub fn daemon_log(&self) -> Utf8PathBuf {
        self.logs().join("daemon.log")
    }

    pub fn launchd_stdout_log(&self) -> Utf8PathBuf {
        self.logs().join("launchd.out.log")
    }

    pub fn launchd_stderr_log(&self) -> Utf8PathBuf {
        self.logs().join("launchd.err.log")
    }

    pub fn downloads(&self) -> &Utf8Path {
        &self.downloads
    }

    pub fn config(&self) -> &Utf8Path {
        &self.config
    }

    pub fn resolver_config(&self) -> Utf8PathBuf {
        self.config().join("resolver/test")
    }

    pub fn pf_anchor_config(&self) -> Utf8PathBuf {
        self.config().join("pf/com.prvious.pv")
    }

    pub fn pf_conf_reference_config(&self) -> Utf8PathBuf {
        self.config().join("pf/pf.conf")
    }

    pub fn gateway_root_config(&self) -> Utf8PathBuf {
        self.config().join("gateway/Caddyfile")
    }

    pub fn gateway_projects_config_dir(&self) -> Utf8PathBuf {
        self.config().join("gateway/projects")
    }

    pub fn worker_root_config(&self, php_track: &str) -> Utf8PathBuf {
        self.config()
            .join(format!("workers/php-{php_track}/Caddyfile"))
    }

    pub fn worker_projects_config_dir(&self, php_track: &str) -> Utf8PathBuf {
        self.config()
            .join(format!("workers/php-{php_track}/projects"))
    }

    pub fn resource_runtime_config(&self, resource_name: &str, track: &str) -> Utf8PathBuf {
        self.config()
            .join(format!("resources/{resource_name}/{track}.json"))
    }

    pub fn certificates(&self) -> &Utf8Path {
        &self.certificates
    }

    pub fn ca_certificate(&self) -> Utf8PathBuf {
        self.certificates().join("ca.pem")
    }

    pub fn ca_private_key(&self) -> Utf8PathBuf {
        self.certificates().join("ca-key.pem")
    }

    pub fn gateway_log(&self) -> Utf8PathBuf {
        self.logs().join("gateway/gateway.log")
    }

    pub fn gateway_access_log(&self) -> Utf8PathBuf {
        self.logs().join("gateway/access.log")
    }

    pub fn gateway_error_log(&self) -> Utf8PathBuf {
        self.logs().join("gateway/error.log")
    }

    pub fn worker_log(&self, php_track: &str) -> Utf8PathBuf {
        self.logs().join(format!("workers/php-{php_track}.log"))
    }

    pub fn resource_log(&self, resource_name: &str, track: &str) -> Utf8PathBuf {
        self.logs()
            .join(format!("resources/{resource_name}/{track}.log"))
    }

    pub fn gateway_pid(&self) -> Utf8PathBuf {
        self.run().join("gateway.pid")
    }

    pub fn gateway_runtime_metadata(&self) -> Utf8PathBuf {
        self.run().join("gateway.json")
    }

    pub fn worker_pid(&self, php_track: &str) -> Utf8PathBuf {
        self.run().join(format!("workers/php-{php_track}.pid"))
    }

    pub fn worker_runtime_metadata(&self, php_track: &str) -> Utf8PathBuf {
        self.run().join(format!("workers/php-{php_track}.json"))
    }

    pub fn resource_pid(&self, resource_name: &str, track: &str) -> Utf8PathBuf {
        self.run()
            .join(format!("resources/{resource_name}/{track}.pid"))
    }

    pub fn resource_runtime_metadata(&self, resource_name: &str, track: &str) -> Utf8PathBuf {
        self.run()
            .join(format!("resources/{resource_name}/{track}.json"))
    }

    pub fn composer(&self) -> &Utf8Path {
        &self.composer
    }

    pub fn resources(&self) -> &Utf8Path {
        &self.resources
    }

    pub fn resource_data_dir(&self, resource_name: &str, track: &str) -> Utf8PathBuf {
        self.resources()
            .join(resource_name)
            .join(track)
            .join("data")
    }

    pub fn layout_directories(&self) -> Vec<(&'static str, &Utf8Path)> {
        self.path_entries()
            .into_iter()
            .filter(|entry| entry.layout_directory)
            .map(|entry| (entry.name, entry.path))
            .collect()
    }

    pub fn summary(&self) -> Vec<PathSummaryEntry> {
        self.path_entries()
            .into_iter()
            .map(|entry| PathSummaryEntry {
                name: entry.name,
                path: entry.path.to_string(),
            })
            .collect()
    }

    fn path_entries(&self) -> [PathEntry<'_>; 11] {
        [
            PathEntry {
                name: "home",
                path: self.home(),
                layout_directory: false,
            },
            PathEntry {
                name: "root",
                path: self.root(),
                layout_directory: true,
            },
            PathEntry {
                name: "db",
                path: self.db(),
                layout_directory: false,
            },
            PathEntry {
                name: "bin",
                path: self.bin(),
                layout_directory: true,
            },
            PathEntry {
                name: "run",
                path: self.run(),
                layout_directory: true,
            },
            PathEntry {
                name: "logs",
                path: self.logs(),
                layout_directory: true,
            },
            PathEntry {
                name: "downloads",
                path: self.downloads(),
                layout_directory: true,
            },
            PathEntry {
                name: "config",
                path: self.config(),
                layout_directory: true,
            },
            PathEntry {
                name: "certificates",
                path: self.certificates(),
                layout_directory: true,
            },
            PathEntry {
                name: "composer",
                path: self.composer(),
                layout_directory: true,
            },
            PathEntry {
                name: "resources",
                path: self.resources(),
                layout_directory: true,
            },
        ]
    }
}
