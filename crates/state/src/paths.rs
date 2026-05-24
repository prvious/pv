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

    pub fn logs(&self) -> &Utf8Path {
        &self.logs
    }

    pub fn downloads(&self) -> &Utf8Path {
        &self.downloads
    }

    pub fn config(&self) -> &Utf8Path {
        &self.config
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
        vec![
            ("root", self.root()),
            ("bin", self.bin()),
            ("run", self.run()),
            ("logs", self.logs()),
            ("downloads", self.downloads()),
            ("config", self.config()),
            ("certificates", self.certificates()),
            ("composer", self.composer()),
            ("resources", self.resources()),
        ]
    }

    pub fn summary(&self) -> Vec<PathSummaryEntry> {
        vec![
            PathSummaryEntry {
                name: "home",
                path: self.home().to_string(),
            },
            PathSummaryEntry {
                name: "root",
                path: self.root().to_string(),
            },
            PathSummaryEntry {
                name: "db",
                path: self.db().to_string(),
            },
            PathSummaryEntry {
                name: "bin",
                path: self.bin().to_string(),
            },
            PathSummaryEntry {
                name: "run",
                path: self.run().to_string(),
            },
            PathSummaryEntry {
                name: "logs",
                path: self.logs().to_string(),
            },
            PathSummaryEntry {
                name: "downloads",
                path: self.downloads().to_string(),
            },
            PathSummaryEntry {
                name: "config",
                path: self.config().to_string(),
            },
            PathSummaryEntry {
                name: "certificates",
                path: self.certificates().to_string(),
            },
            PathSummaryEntry {
                name: "composer",
                path: self.composer().to_string(),
            },
            PathSummaryEntry {
                name: "resources",
                path: self.resources().to_string(),
            },
        ]
    }
}
