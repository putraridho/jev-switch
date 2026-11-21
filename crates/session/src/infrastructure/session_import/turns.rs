//! Builds turns from a stream of user prompts, answers and tool calls, whichever tool wrote them.

use jev_backend::ProviderId;
use crate::domain::turn_record::TurnRecord;

/// Builds turns from a stream of user prompts, answers and tool calls.
pub(super) struct TurnBuilder {
    provider: ProviderId,
    done: Vec<TurnRecord>,
    open: Option<TurnRecord>,
}

impl TurnBuilder {
    pub(super) fn new(provider: ProviderId) -> Self {
        Self { provider, done: Vec::new(), open: None }
    }

    pub(super) fn prompt(&mut self, text: &str) {
        self.close();
        self.open = Some(TurnRecord { prompt: text.trim().to_string(), provider: self.provider, model: String::new(), reply: String::new(), tools: Vec::new(), ok: true });
    }

    pub(super) fn answer(&mut self, text: &str, model: Option<&str>) {
        if let Some(t) = self.open.as_mut() {
            if !text.trim().is_empty() {
                t.reply = text.trim().to_string();
            }
            if let Some(m) = model.filter(|m| !m.is_empty()) {
                t.model = m.to_string();
            }
        }
    }

    pub(super) fn tool(&mut self, summary: String) {
        if let Some(t) = self.open.as_mut() {
            t.tools.push(summary);
        }
    }

    fn close(&mut self) {
        self.done.extend(self.open.take());
    }

    pub(super) fn finish(mut self) -> Vec<TurnRecord> {
        self.close();
        self.done
    }
}

/// A "user" message the person did not type: injected context and command wrappers.
pub(super) fn is_injected(text: &str) -> bool {
    let t = text.trim_start();
    t.is_empty() || t.starts_with('<') || t.starts_with("Caveat:") || t.starts_with("[Request interrupted")
}

pub(super) fn tool_summary(name: &str, arg: Option<&str>) -> String {
    match arg.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("{name} {}", a.lines().next().unwrap_or_default()),
        None => name.to_string(),
    }
}
