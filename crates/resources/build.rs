use std::io::Write;

fn main() -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(
        stdout,
        "cargo:rerun-if-env-changed=PV_DEFAULT_ARTIFACT_MANIFEST_URL"
    )
}
