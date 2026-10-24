use std::io;

use camino::Utf8Path;
use serde::Serialize;
use state::fs;

use crate::environment::Environment;
use crate::output::Output;

#[derive(Serialize)]
pub(super) struct LinuxDistribution {
    pretty_name: String,
    support: &'static str,
}

impl LinuxDistribution {
    pub(super) fn read(environment: &impl Environment) -> Self {
        let path = environment.os_release_path();
        let pretty_name = Utf8Path::from_path(&path)
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|content| {
                let value = content
                    .lines()
                    .filter_map(|line| line.strip_prefix("PRETTY_NAME="))
                    .next_back()?;
                let mut words = shell_words::split(value).ok()?.into_iter();
                let name = words.next()?;
                (!name.is_empty() && !name.chars().any(char::is_control) && words.next().is_none())
                    .then_some(name)
            })
            .unwrap_or_else(|| "Linux".to_owned());
        Self {
            pretty_name,
            support: "untested",
        }
    }

    pub(super) fn write(&self, output: &mut Output<'_>) -> io::Result<()> {
        output.line(&format!(
            "Linux distribution: {} ({})",
            self.pretty_name, self.support
        ))
    }
}
