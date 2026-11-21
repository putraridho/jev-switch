//! The message being typed during a turn, and what each key does to it.

use super::keys::Key;

/// What a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Effect {
    Nothing,
    /// The draft changed: redraw it.
    Changed,
    /// Enter: this message joins the queue.
    Queued(String),
    /// Esc: stop the turn, and send this message first when one was typed.
    Stop(Option<String>),
}

/// The message being typed during a turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Draft {
    pub(super) text: String,
}

impl Draft {
    pub(super) fn apply(&mut self, key: Key) -> Effect {
        match key {
            Key::Text(t) => {
                self.text.push_str(&t);
                Effect::Changed
            }
            Key::Newline => {
                self.text.push('\n');
                Effect::Changed
            }
            Key::Backspace => Self::changed(self.text.pop().is_some()),
            Key::ClearLine => Self::changed(!std::mem::take(&mut self.text).is_empty()),
            Key::DeleteWord => {
                let before = self.text.len();
                let kept = self.text.trim_end().trim_end_matches(|c: char| !c.is_whitespace()).len();
                self.text.truncate(kept);
                Self::changed(kept != before)
            }
            Key::Enter => {
                if self.text.trim().is_empty() {
                    self.text.clear();
                    Effect::Nothing
                } else {
                    Effect::Queued(std::mem::take(&mut self.text).trim().to_string())
                }
            }
            Key::Esc => {
                let typed = std::mem::take(&mut self.text);
                Effect::Stop((!typed.trim().is_empty()).then(|| typed.trim().to_string()))
            }
        }
    }

    fn changed(yes: bool) -> Effect {
        if yes {
            Effect::Changed
        } else {
            Effect::Nothing
        }
    }
}
