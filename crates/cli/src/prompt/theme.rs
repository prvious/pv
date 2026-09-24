use cliclack::{Theme, ThemeState};
use console::Style;

use super::{Prompt, PromptKind};
use crate::output::Tone;

/// The terminal design's prompt treatment: `◆` active and `◇` answered
/// markers, a dim `│` gutter, `●`/`○` radios, `[x]`/`[ ]` checkboxes, and a
/// key hint in the footer. Colors share the output rows' tones;
/// `Environment::set_terminal_colors` disables them when stderr has no color
/// (`NO_COLOR`, `--no-color`, or no terminal).
pub(super) struct PvTheme {
    hint: &'static str,
    /// Whether the prompt continues an open flow and opens with a `│` line.
    joined: bool,
}

impl PvTheme {
    pub(super) fn for_prompt(prompt: &Prompt<'_>) -> Self {
        let hint = match prompt.kind {
            PromptKind::Confirm { .. } => "y yes · n no · enter accepts the highlighted answer",
            PromptKind::Select { .. } => "↑↓ move · enter confirm",
            PromptKind::MultiSelect { .. } => "↑↓ move · space toggle · enter confirm",
            PromptKind::Text { default: "", .. } => "enter submit",
            PromptKind::Text { .. } => "enter submit · empty keeps the default",
        };

        Self {
            hint,
            joined: prompt.joined,
        }
    }
}

impl Theme for PvTheme {
    fn bar_color(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Error(_) => Tone::Warning.console(),
            ThemeState::Cancel => Tone::Failure.console(),
            ThemeState::Active | ThemeState::Submit => Tone::Dim.console(),
        }
    }

    fn state_symbol_color(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Active => Tone::Accent.console(),
            ThemeState::Submit => Tone::Success.console(),
            ThemeState::Cancel => Tone::Failure.console(),
            ThemeState::Error(_) => Tone::Warning.console(),
        }
    }

    fn state_symbol(&self, state: &ThemeState) -> String {
        let symbol = match state {
            ThemeState::Active | ThemeState::Error(_) => "◆",
            ThemeState::Submit => "◇",
            ThemeState::Cancel => "✗",
        };

        self.state_symbol_color(state).apply_to(symbol).to_string()
    }

    fn radio_symbol(&self, state: &ThemeState, selected: bool) -> String {
        match state {
            ThemeState::Active | ThemeState::Error(_) if selected => {
                Tone::Value.console().apply_to("●").to_string()
            }
            ThemeState::Active | ThemeState::Error(_) => {
                Tone::Dim.console().apply_to("○").to_string()
            }
            ThemeState::Submit | ThemeState::Cancel => String::new(),
        }
    }

    /// Shows every choice's hint, not just the highlighted one's, so a picker
    /// lists each Project's path beside its hostname. Answered and cancelled
    /// prompts keep Cliclack's own label styling.
    fn radio_item(&self, state: &ThemeState, selected: bool, label: &str, hint: &str) -> String {
        match state {
            ThemeState::Cancel | ThemeState::Submit if !selected => return String::new(),
            ThemeState::Cancel | ThemeState::Submit => {
                return self.input_style(state).apply_to(label).to_string();
            }
            ThemeState::Active | ThemeState::Error(_) => {}
        }
        let label = if selected {
            self.input_style(state).apply_to(label)
        } else {
            self.placeholder_style(state).apply_to(label)
        };
        let hint = if hint.is_empty() {
            String::new()
        } else {
            format!("  {}", self.placeholder_style(state).apply_to(hint))
        };

        format!("{} {label}{hint}", self.radio_symbol(state, selected))
    }

    fn checkbox_symbol(&self, state: &ThemeState, selected: bool, active: bool) -> String {
        let (tone, symbol) = match state {
            ThemeState::Submit | ThemeState::Cancel => return String::new(),
            ThemeState::Active | ThemeState::Error(_) if selected => (Tone::Success, "[x]"),
            ThemeState::Active | ThemeState::Error(_) if active => (Tone::Value, "[ ]"),
            ThemeState::Active | ThemeState::Error(_) => (Tone::Dim, "[ ]"),
        };

        tone.console().apply_to(symbol).to_string()
    }

    /// Opens a prompt that continues a flow with a gutter line, so it joins
    /// the rows above with exactly one `│` spacer.
    fn format_header(&self, state: &ThemeState, prompt: &str) -> String {
        let bar = self.bar_color(state).apply_to("│");
        let mut header = if self.joined {
            format!("{bar}\n")
        } else {
            String::new()
        };
        for (index, line) in prompt.lines().enumerate() {
            if index == 0 {
                header.push_str(&format!("{}  {line}\n", self.state_symbol(state)));
            } else {
                header.push_str(&format!("{bar}  {line}\n"));
            }
        }

        header
    }

    fn format_footer_with_message(&self, state: &ThemeState, message: &str) -> String {
        let bar = self.bar_color(state);
        let line = match state {
            ThemeState::Active => {
                let hint = if message.is_empty() {
                    self.hint.to_string()
                } else {
                    format!("{message}  {}", self.hint)
                };
                format!(
                    "{}  {}",
                    bar.apply_to("└"),
                    Tone::Dim.console().apply_to(hint)
                )
            }
            ThemeState::Cancel => format!("{}  Cancelled.", bar.apply_to("└")),
            // An answered prompt ends at its value; the next row or prompt
            // opens with its own spacer.
            ThemeState::Submit => return String::new(),
            ThemeState::Error(error) => format!("{}  {error}", bar.apply_to("└")),
        };

        format!("{line}\n")
    }
}
