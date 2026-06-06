use camino::Utf8Path;

const BLOCKED_PATHS: &[&str] = &["/opt/homebrew", "/usr/local/Cellar", "/Users/runner"];

pub fn scan_relocation_text(path: &Utf8Path, text: &str) -> crate::Result<()> {
    for blocked in BLOCKED_PATHS {
        if text.contains(blocked) {
            return Err(crate::ReleaseError::Relocation {
                path: path.to_string(),
                reason: format!("blocked runtime path `{blocked}`"),
            });
        }
    }

    Ok(())
}

pub fn scan_file(path: &Utf8Path) -> crate::Result<()> {
    let bytes = read_file(path)?;
    let text = String::from_utf8_lossy(&bytes);
    scan_relocation_text(path, &text)
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling scans local artifact files for blocked runtime paths"
)]
fn read_file(path: &Utf8Path) -> crate::Result<Vec<u8>> {
    std::fs::read(path).map_err(|error| crate::ReleaseError::Filesystem {
        path: path.to_string(),
        reason: error.to_string(),
    })
}
