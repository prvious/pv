//! Gives each pv-fake build an ID from its sources, so an installed fake can tell when it is older
//! than the library that installed it, e.g. after `cargo nextest run --test …`, which doesn't
//! rebuild the daemon's examples.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::Path;

#[expect(
    clippy::disallowed_macros,
    reason = "build scripts report to Cargo on stdout"
)]
fn main() -> io::Result<()> {
    let mut hasher = DefaultHasher::new();
    hash_directory(Path::new("src"), &mut hasher)?;
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rustc-env=PV_FAKE_BUILD_ID={:016x}", hasher.finish());

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "build script reads its own crate sources"
)]
fn hash_directory(directory: &Path, hasher: &mut DefaultHasher) -> io::Result<()> {
    let mut entries = std::fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort();
    for path in entries {
        path.hash(hasher);
        if path.is_dir() {
            hash_directory(&path, hasher)?;
        } else {
            std::fs::read(&path)?.hash(hasher);
        }
    }

    Ok(())
}
