use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use state::{PvPaths, StateError, fs};

pub const PHP_TRACK_DEFAULT_INI: &str = include_str!("php-defaults.ini");
const SUPPORTED_PHP_TRACKS: [&str; 3] = ["8.3", "8.4", "8.5"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpTrackDefaults {
    etc_dir: Utf8PathBuf,
    php_ini: Utf8PathBuf,
    conf_dir: Utf8PathBuf,
}

impl PhpTrackDefaults {
    pub fn etc_dir(&self) -> &Utf8Path {
        &self.etc_dir
    }

    pub fn php_ini(&self) -> &Utf8Path {
        &self.php_ini
    }

    pub fn conf_dir(&self) -> &Utf8Path {
        &self.conf_dir
    }
}

pub fn php_track_defaults(paths: &PvPaths, track: &str) -> PhpTrackDefaults {
    let etc_dir = paths.resources().join(format!("php/{track}/etc"));
    let php_ini = etc_dir.join("php.ini");
    let conf_dir = etc_dir.join("conf.d");

    PhpTrackDefaults {
        etc_dir,
        php_ini,
        conf_dir,
    }
}

pub fn ensure_php_track_defaults(
    paths: &PvPaths,
    track: &str,
) -> Result<PhpTrackDefaults, StateError> {
    ensure_supported_track(track)?;
    let defaults = php_track_defaults(paths, track);

    ensure_directory_path(defaults.etc_dir(), "etc")?;
    ensure_directory_path(defaults.conf_dir(), "conf.d")?;
    ensure_php_ini_path(defaults.php_ini())?;

    Ok(defaults)
}

pub fn php_track_environment(paths: &PvPaths, track: &str) -> BTreeMap<String, String> {
    let defaults = php_track_defaults(paths, track);

    BTreeMap::from([
        ("PHPRC".to_owned(), defaults.etc_dir().to_string()),
        (
            "PHP_INI_SCAN_DIR".to_owned(),
            defaults.conf_dir().to_string(),
        ),
    ])
}

pub fn php_track_exec_environment(paths: &PvPaths, track: &str) -> Vec<(OsString, OsString)> {
    php_track_environment(paths, track)
        .into_iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect()
}

fn ensure_directory_path(path: &Utf8Path, name: &'static str) -> Result<(), StateError> {
    if path.exists() && !path.is_dir() {
        return Err(StateError::Filesystem {
            path: path.to_path_buf(),
            source: io::Error::other(format!("PHP track defaults {name} path is not a directory")),
        });
    }

    fs::ensure_user_dir(path)
}

fn ensure_php_ini_path(path: &Utf8Path) -> Result<(), StateError> {
    if path.exists() && !path.is_file() {
        return Err(StateError::Filesystem {
            path: path.to_path_buf(),
            source: io::Error::other("PHP track defaults php.ini path is not a file"),
        });
    }

    if path.exists() {
        let _content = fs::read_to_string(path)?;
        return Ok(());
    }

    fs::write_sensitive_file(path, PHP_TRACK_DEFAULT_INI)
}

fn ensure_supported_track(track: &str) -> Result<(), StateError> {
    if SUPPORTED_PHP_TRACKS.contains(&track) {
        return Ok(());
    }

    Err(StateError::InvalidProjectTrack {
        track: track.to_owned(),
    })
}
