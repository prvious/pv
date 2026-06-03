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

    pub fn run(&self) -> &Utf8Path {
        &self.run
    }

    pub fn daemon_socket(&self) -> Utf8PathBuf {
        self.run().join("pv.sock")
    }

    pub fn logs(&self) -> &Utf8Path {
        &self.logs
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

    pub fn certificates(&self) -> &Utf8Path {
        &self.certificates
    }

    pub fn composer(&self) -> &Utf8Path {
        &self.composer
    }

    pub fn resources(&self) -> &Utf8Path {
        &self.resources
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
