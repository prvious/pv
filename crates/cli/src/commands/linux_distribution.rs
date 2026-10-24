use std::io;

use camino::Utf8Path;
use serde::Serialize;
use state::fs;

use crate::environment::Environment;
use crate::output::Output;

const SUPPORTED_DISTRIBUTIONS: &str =
    include_str!("../../../../release/linux-supported-distributions.txt");

#[derive(Serialize)]
pub(super) struct LinuxDistribution {
    pretty_name: String,
    support: &'static str,
}

impl LinuxDistribution {
    pub(super) fn read(environment: &impl Environment) -> Self {
        let path = environment.os_release_path();
        let content = Utf8Path::from_path(&path)
            .and_then(|path| fs::read_to_string(path).ok())
            .unwrap_or_default();
        let field = |prefix| {
            let value = content
                .lines()
                .filter_map(|line| line.strip_prefix(prefix))
                .next_back()?;
            let mut words = shell_words::split(value).ok()?.into_iter();
            let name = words.next()?;
            (!name.is_empty() && !name.chars().any(char::is_control) && words.next().is_none())
                .then_some(name)
        };
        let supported = field("ID=")
            .zip(field("VERSION_ID="))
            .zip(environment.resolve_target_platform().ok())
            .is_some_and(|((id, version), platform)| {
                let pair = format!("{id}:{version}:{platform}");
                SUPPORTED_DISTRIBUTIONS.lines().any(|line| line == pair)
            });
        Self {
            pretty_name: field("PRETTY_NAME=").unwrap_or_else(|| "Linux".to_owned()),
            support: if supported { "supported" } else { "untested" },
        }
    }

    pub(super) fn write(&self, output: &mut Output<'_>) -> io::Result<()> {
        output.line(&format!(
            "Linux distribution: {} ({})",
            self.pretty_name, self.support
        ))
    }
}
