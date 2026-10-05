//! One canonical conversation history. Sent in full to whichever model a turn
//! routes to — this is what keeps memory intact across model switches.

use crate::anthropic::Message;

pub struct Transcript {
    pub system: String,
    pub messages: Vec<Message>,
}

impl Transcript {
    pub fn new(system: impl Into<String>) -> Self {
        Self { system: system.into(), messages: Vec::new() }
    }

    pub fn push_user(&mut self, text: impl Into<String>) {
        self.messages.push(Message::user(text));
    }

    pub fn push_assistant(&mut self, text: impl Into<String>) {
        self.messages.push(Message::assistant(text));
    }

    /// A compact tail of recent turns, for the router's `recent_context` state.
    pub fn recent_context(&self, turns: usize) -> String {
        let start = self.messages.len().saturating_sub(turns);
        self.messages[start..]
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
