use std::io;

use camino::{Utf8Path, Utf8PathBuf};

pub const SYSTEM_PF_ANCHOR_PATH: &str = "/etc/pf.anchors/com.prvious.pv";
pub const SYSTEM_PF_CONF_PATH: &str = "/etc/pf.conf";
const PV_MARKER: &str = "# Managed by PV";
const PF_ANCHOR_SOURCE_MARKER: &str =
    "# Source: PV prepared pf anchor for /etc/pf.anchors/com.prvious.pv";
const PF_CONF_SOURCE_MARKER: &str = "# Source: PV prepared pf.conf reference for /etc/pf.conf";
const PF_ANCHOR_DIRECTIVE: &str = "anchor \"com.prvious.pv\"";
const PF_LOAD_ANCHOR_DIRECTIVE: &str =
    "load anchor \"com.prvious.pv\" from \"/etc/pf.anchors/com.prvious.pv\"";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PfRedirectConfig {
    pub http_port: u16,
    pub https_port: u16,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct PfConfReference;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PfFileState<T> {
    Missing {
        path: Utf8PathBuf,
    },
    Current {
        path: Utf8PathBuf,
        value: T,
    },
    Stale {
        path: Utf8PathBuf,
        expected: Option<T>,
        actual: Option<T>,
    },
    Conflict {
        path: Utf8PathBuf,
    },
    Unreadable {
        path: Utf8PathBuf,
        message: String,
    },
}

impl PfRedirectConfig {
    pub const fn new(http_port: u16, https_port: u16) -> Self {
        Self {
            http_port,
            https_port,
        }
    }

    pub fn render_anchor(&self) -> String {
        format!(
            "{PV_MARKER}\n{PF_ANCHOR_SOURCE_MARKER}\nrdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port {}\nrdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port {}\n",
            self.http_port, self.https_port
        )
    }

    pub fn parse_anchor(content: &str) -> Option<Self> {
        let mut http_port = None;
        let mut https_port = None;
        let mut active_line_count = 0;

        for line in content.lines().filter_map(active_pf_line) {
            active_line_count += 1;

            if let Some(port) = line.strip_prefix(
                "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 80 -> 127.0.0.1 port ",
            ) {
                if http_port.replace(port.parse::<u16>().ok()?).is_some() {
                    return None;
                }
                continue;
            }

            if let Some(port) = line.strip_prefix(
                "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port ",
            ) {
                if https_port.replace(port.parse::<u16>().ok()?).is_some() {
                    return None;
                }
                continue;
            }

            return None;
        }

        if active_line_count == 2 {
            Some(Self::new(http_port?, https_port?))
        } else {
            None
        }
    }
}

impl PfConfReference {
    pub fn render(self) -> String {
        format!(
            "{PV_MARKER}\n{PF_CONF_SOURCE_MARKER}\n{PF_ANCHOR_DIRECTIVE}\n{PF_LOAD_ANCHOR_DIRECTIVE}\n"
        )
    }

    pub fn parse_block(content: &str) -> Option<Self> {
        let mut has_anchor = false;
        let mut has_load = false;
        let mut active_line_count = 0;

        for line in content.lines().filter_map(active_pf_line) {
            active_line_count += 1;

            if line == PF_ANCHOR_DIRECTIVE {
                if has_anchor {
                    return None;
                }
                has_anchor = true;
                continue;
            }

            if line == PF_LOAD_ANCHOR_DIRECTIVE {
                if has_load {
                    return None;
                }
                has_load = true;
                continue;
            }

            if is_pv_pf_conf_reference_directive(line) {
                return None;
            }

            return None;
        }

        if active_line_count == 2 && has_anchor && has_load {
            Some(Self)
        } else {
            None
        }
    }
}

pub fn inspect_pf_anchor_file(
    path: &Utf8Path,
    expected: Option<&PfRedirectConfig>,
) -> PfFileState<PfRedirectConfig> {
    inspect_pv_file(path, expected, PfRedirectConfig::parse_anchor, true)
}

pub fn inspect_pf_conf_reference(
    path: &Utf8Path,
    expected: Option<&PfConfReference>,
) -> PfFileState<PfConfReference> {
    let content = match state::fs::read_to_string(path) {
        Ok(content) => content,
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return PfFileState::Missing {
                path: path.to_path_buf(),
            };
        }
        Err(error) => {
            return PfFileState::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    let has_pv_marker = content.lines().any(|line| line.trim() == PV_MARKER);
    let has_anchor_directive = content
        .lines()
        .filter_map(active_pf_line)
        .any(is_pv_pf_conf_reference_directive);

    if !has_pv_marker {
        return if has_anchor_directive {
            PfFileState::Conflict {
                path: path.to_path_buf(),
            }
        } else {
            PfFileState::Missing {
                path: path.to_path_buf(),
            }
        };
    }

    let actual = parse_embedded_pf_conf_reference(&content);
    classify_pv_file_state(path, expected, actual)
}

fn inspect_pv_file<T: Clone + Eq>(
    path: &Utf8Path,
    expected: Option<&T>,
    parse: impl FnOnce(&str) -> Option<T>,
    conflict_when_unmarked: bool,
) -> PfFileState<T> {
    let content = match state::fs::read_to_string(path) {
        Ok(content) => content,
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return PfFileState::Missing {
                path: path.to_path_buf(),
            };
        }
        Err(error) => {
            return PfFileState::Unreadable {
                path: path.to_path_buf(),
                message: error.to_string(),
            };
        }
    };

    if !content.lines().any(|line| line.trim() == PV_MARKER) && conflict_when_unmarked {
        return PfFileState::Conflict {
            path: path.to_path_buf(),
        };
    }

    let actual = parse(&content);
    classify_pv_file_state(path, expected, actual)
}

fn classify_pv_file_state<T: Clone + Eq>(
    path: &Utf8Path,
    expected: Option<&T>,
    actual: Option<T>,
) -> PfFileState<T> {
    match (expected, actual) {
        (Some(expected), Some(actual)) if expected == &actual => PfFileState::Current {
            path: path.to_path_buf(),
            value: actual,
        },
        (Some(expected), actual) => PfFileState::Stale {
            path: path.to_path_buf(),
            expected: Some(expected.clone()),
            actual,
        },
        (None, Some(actual)) => PfFileState::Current {
            path: path.to_path_buf(),
            value: actual,
        },
        (None, None) => PfFileState::Stale {
            path: path.to_path_buf(),
            expected: None,
            actual: None,
        },
    }
}

fn active_pf_line(line: &str) -> Option<&str> {
    let line = line.trim();

    if line.is_empty() || line.starts_with('#') {
        None
    } else {
        Some(line)
    }
}

fn is_pv_pf_conf_reference_directive(line: &str) -> bool {
    line.starts_with("anchor \"com.prvious.pv\"")
        || line.starts_with("load anchor \"com.prvious.pv\"")
}

fn parse_embedded_pf_conf_reference(content: &str) -> Option<PfConfReference> {
    let mut has_anchor = false;
    let mut has_load = false;

    for line in content.lines().filter_map(active_pf_line) {
        if line == PF_ANCHOR_DIRECTIVE {
            if has_anchor {
                return None;
            }
            has_anchor = true;
            continue;
        }

        if line == PF_LOAD_ANCHOR_DIRECTIVE {
            if has_load {
                return None;
            }
            has_load = true;
            continue;
        }

        if is_pv_pf_conf_reference_directive(line) {
            return None;
        }
    }

    if has_anchor && has_load {
        Some(PfConfReference)
    } else {
        None
    }
}
