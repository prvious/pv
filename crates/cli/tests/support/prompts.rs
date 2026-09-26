use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;

use cli::{Answer, Prompt, PromptKind};

/// One scripted reaction to a prompt.
#[derive(Clone, Debug)]
pub(crate) enum Step {
    /// Press Enter on the shown default.
    Accept,
    Answer(Answer),
    /// Press Escape or Ctrl-C.
    Cancel,
}

/// Answers keyboard prompts from a script and records what was asked, so
/// prompt decisions are testable without a terminal.
///
/// Text answers that fail the prompt's validator are recorded as rejected and
/// the next step answers the same prompt again, as the real prompt does.
#[derive(Debug, Default)]
pub(crate) struct ScriptedPrompts {
    steps: RefCell<VecDeque<Step>>,
    transcript: RefCell<Vec<String>>,
}

impl ScriptedPrompts {
    pub(crate) fn new(steps: impl IntoIterator<Item = Step>) -> Self {
        Self {
            steps: RefCell::new(steps.into_iter().collect()),
            transcript: RefCell::default(),
        }
    }

    pub(crate) fn ask(&self, prompt: &Prompt<'_>) -> io::Result<Answer> {
        loop {
            let step = self.steps.borrow_mut().pop_front().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("no scripted answer for `{}`", prompt.message),
                )
            })?;
            let answer = match step {
                Step::Accept => default_answer(prompt),
                Step::Answer(answer) => answer,
                Step::Cancel => Answer::Cancelled,
            };
            if let (PromptKind::Text { validator, .. }, Answer::Text(text)) =
                (&prompt.kind, &answer)
                && let Some(validator) = validator
                && let Err(error) = validator(text)
            {
                self.record(prompt, &format!("rejected {text:?}: {error}"));
                continue;
            }
            self.record(prompt, &format!("{answer:?}"));

            return Ok(answer);
        }
    }

    /// Every prompt asked, in order, with its outcome.
    pub(crate) fn transcript(&self) -> Vec<String> {
        self.transcript.borrow().clone()
    }

    fn record(&self, prompt: &Prompt<'_>, outcome: &str) {
        self.transcript
            .borrow_mut()
            .push(format!("{} -> {outcome}", prompt.message));
    }
}

fn default_answer(prompt: &Prompt<'_>) -> Answer {
    match prompt.kind {
        PromptKind::Confirm { default } => Answer::Confirmed(default),
        PromptKind::Select { initial, .. } => Answer::Selected(initial),
        PromptKind::MultiSelect { selected, .. } => Answer::SelectedMany(selected.to_vec()),
        PromptKind::Text { default, .. } => Answer::Text(default.to_string()),
    }
}
