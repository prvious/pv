use std::io;

use camino::Utf8Path;
use serde::Serialize;
use state::fs;

use crate::environment::Environment;
use crate::output::Output;

#[derive(Serialize)]
pub(super) struct LinuxDistribution {
    pretty_name: String,
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
                let name = value
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .or_else(|| {
                        value
                            .strip_prefix('\'')
                            .and_then(|value| value.strip_suffix('\''))
                    })
                    .unwrap_or(value);
                (!name.is_empty() && !name.chars().any(char::is_control)).then(|| name.to_owned())
            })
            .unwrap_or_else(|| "Linux".to_owned());
        Self { pretty_name }
    }

    pub(super) fn write(&self, output: &mut Output<'_>) -> io::Result<()> {
        output.line(&format!("Linux distribution: {}", self.pretty_name))
    }
}
