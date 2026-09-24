//! Keyboard prompts behind a PV-owned adapter.
//!
//! Commands describe a [`Prompt`] and receive a PV [`Answer`]; only this
//! module knows about Cliclack. Prompts render on stderr and require
//! stdin and stderr to be terminals, which callers check through
//! `Streams::interactive` before prompting, because each command documents
//! its own non-interactive behavior.

use std::io;

use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, Streams};

mod theme;

/// A validator for text answers; the error is shown inline and the prompt
/// asks again.
pub type Validator = fn(&str) -> Result<(), String>;

#[derive(Debug)]
pub struct Prompt<'a> {
    pub message: &'a str,
    /// Whether the prompt continues an open `┌ │ └` flow on the terminal.
    pub joined: bool,
    pub kind: PromptKind<'a>,
}

#[derive(Debug)]
pub enum PromptKind<'a> {
    Confirm {
        default: bool,
    },
    Select {
        choices: &'a [Choice<'a>],
        initial: usize,
    },
    MultiSelect {
        choices: &'a [Choice<'a>],
        selected: &'a [usize],
    },
    Text {
        default: &'a str,
        validator: Option<Validator>,
    },
}

#[derive(Debug)]
pub struct Choice<'a> {
    pub label: &'a str,
    pub hint: &'a str,
}

impl<'a> Choice<'a> {
    pub(crate) const fn new(label: &'a str, hint: &'a str) -> Self {
        Self { label, hint }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Answer {
    Confirmed(bool),
    Selected(usize),
    SelectedMany(Vec<usize>),
    Text(String),
    /// The user pressed Escape or Ctrl-C.
    Cancelled,
}

pub(crate) fn confirm(
    environment: &impl Environment,
    output: &Output<'_>,
    message: &str,
    default: bool,
) -> Result<bool, ExecuteError> {
    match ask(
        environment,
        output,
        message,
        PromptKind::Confirm { default },
    )? {
        Answer::Confirmed(confirmed) => Ok(confirmed),
        answer => Err(unexpected(&answer)),
    }
}

pub(crate) fn select(
    environment: &impl Environment,
    output: &Output<'_>,
    message: &str,
    choices: &[Choice<'_>],
    initial: usize,
) -> Result<usize, ExecuteError> {
    match ask(
        environment,
        output,
        message,
        PromptKind::Select { choices, initial },
    )? {
        Answer::Selected(index) if index < choices.len() => Ok(index),
        answer => Err(unexpected(&answer)),
    }
}

pub(crate) fn multiselect(
    environment: &impl Environment,
    output: &Output<'_>,
    message: &str,
    choices: &[Choice<'_>],
    selected: &[usize],
) -> Result<Vec<usize>, ExecuteError> {
    match ask(
        environment,
        output,
        message,
        PromptKind::MultiSelect { choices, selected },
    )? {
        Answer::SelectedMany(indexes) if indexes.iter().all(|index| *index < choices.len()) => {
            Ok(indexes)
        }
        answer => Err(unexpected(&answer)),
    }
}

/// Asks for text. The answer is trimmed, and a blank one returns `default`.
pub(crate) fn text(
    environment: &impl Environment,
    output: &Output<'_>,
    message: &str,
    default: &str,
    validator: Option<Validator>,
) -> Result<String, ExecuteError> {
    match ask(
        environment,
        output,
        message,
        PromptKind::Text { default, validator },
    )? {
        // Cliclack fills in the default only for an empty answer, not a
        // blank one.
        Answer::Text(text) if text.trim().is_empty() => Ok(default.to_string()),
        Answer::Text(text) => Ok(text.trim().to_string()),
        answer => Err(unexpected(&answer)),
    }
}

/// Asks a confirmation, or fails with `refusal` when stdin and stderr are not
/// both terminals. Each command chooses a refusal that names its rerun flag.
pub(crate) fn confirm_or(
    environment: &impl Environment,
    streams: &Streams<'_>,
    refusal: CliError,
    message: &str,
    default: bool,
) -> Result<bool, ExecuteError> {
    if !streams.interactive {
        return Err(refusal.into());
    }

    confirm(environment, &streams.out, message, default)
}

fn ask(
    environment: &impl Environment,
    output: &Output<'_>,
    message: &str,
    kind: PromptKind<'_>,
) -> Result<Answer, ExecuteError> {
    let prompt = Prompt {
        message,
        joined: output.in_flow(),
        kind,
    };
    match environment.prompt(&prompt)? {
        Answer::Cancelled => Err(CliError::PromptCancelled.into()),
        answer => Ok(answer),
    }
}

fn unexpected(answer: &Answer) -> ExecuteError {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("prompt returned an unexpected answer: {answer:?}"),
    )
    .into()
}

/// Asks `prompt` on the process terminal through Cliclack.
pub(crate) fn interact(prompt: &Prompt<'_>) -> io::Result<Answer> {
    cliclack::set_theme(theme::PvTheme::for_prompt(prompt));
    let message = prompt.message;
    let answer = match prompt.kind {
        PromptKind::Confirm { default } => cliclack::confirm(message)
            .initial_value(default)
            .interact()
            .map(Answer::Confirmed),
        PromptKind::Select { choices, initial } => {
            let mut select = cliclack::select(message).initial_value(initial);
            for (index, choice) in choices.iter().enumerate() {
                select = select.item(index, choice.label, choice.hint);
            }
            select.interact().map(Answer::Selected)
        }
        PromptKind::MultiSelect { choices, selected } => {
            let mut multiselect = cliclack::multiselect(message)
                .initial_values(selected.to_vec())
                .required(false);
            for (index, choice) in choices.iter().enumerate() {
                multiselect = multiselect.item(index, choice.label, choice.hint);
            }
            multiselect.interact().map(Answer::SelectedMany)
        }
        PromptKind::Text { default, validator } => {
            let mut input = cliclack::input(message);
            input = if default.is_empty() {
                input.required(false)
            } else {
                input.default_input(default)
            };
            if let Some(validator) = validator {
                // The answer is trimmed, so validate what will be used.
                input = input.validate(move |value: &String| validator(value.trim()));
            }
            input.interact::<String>().map(Answer::Text)
        }
    };

    // Cliclack reports Escape and Ctrl-C as `Interrupted`, after drawing the
    // cancelled state and restoring the cursor.
    match answer {
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(Answer::Cancelled),
        answer => answer,
    }
}
