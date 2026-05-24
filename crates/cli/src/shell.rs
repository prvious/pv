use std::ffi::OsStr;
use std::path::Path;

use clap::ValueEnum;

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum Shell {
    Bash,
    Fish,
    Zsh,
}

impl Shell {
    pub(crate) fn detect(shell_path: &OsStr) -> Option<Self> {
        let file_name = Path::new(shell_path).file_name()?;
        let shell_name = file_name.to_string_lossy();

        match shell_name.as_ref() {
            "bash" => Some(Self::Bash),
            "fish" => Some(Self::Fish),
            "zsh" => Some(Self::Zsh),
            _ => None,
        }
    }

    pub(crate) fn env_script(self) -> &'static str {
        match self {
            Self::Bash | Self::Zsh => POSIX_ENV_SCRIPT,
            Self::Fish => FISH_ENV_SCRIPT,
        }
    }

    pub(crate) fn completion_shell(self) -> clap_complete::Shell {
        match self {
            Self::Bash => clap_complete::Shell::Bash,
            Self::Fish => clap_complete::Shell::Fish,
            Self::Zsh => clap_complete::Shell::Zsh,
        }
    }
}

const POSIX_ENV_SCRIPT: &str = r#"pv_prepend_path() {
  case ":$PATH:" in
    *":$1:"*) ;;
    *) PATH="$1${PATH:+:$PATH}" ;;
  esac
}

export COMPOSER_HOME="$HOME/.pv/composer"
export COMPOSER_CACHE_DIR="$HOME/.pv/composer/cache"
pv_prepend_path "$HOME/.pv/composer/vendor/bin"
pv_prepend_path "$HOME/.pv/bin"
export PATH
unset -f pv_prepend_path
"#;

const FISH_ENV_SCRIPT: &str = r#"set -gx COMPOSER_HOME "$HOME/.pv/composer"
set -gx COMPOSER_CACHE_DIR "$HOME/.pv/composer/cache"
contains -- "$HOME/.pv/composer/vendor/bin" $PATH; or set -gx PATH "$HOME/.pv/composer/vendor/bin" $PATH
contains -- "$HOME/.pv/bin" $PATH; or set -gx PATH "$HOME/.pv/bin" $PATH
"#;
