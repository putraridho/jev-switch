//! What Jev said about one turn.

use serde_json::Value;

use crate::judgment::domain::want::{CodeWant, ProviderWant};
use crate::routing::domain::decide::Signals;
/// What Jev (or the legacy tier judgment) said about one turn.
pub enum Judged {
    /// Provider routing; `None` when Jev was unavailable.
    Providers(Option<ProviderWant>),
    /// Routing off: the old Claude-only tier judgment.
    Legacy(CodeWant),
}

impl Judged {
    pub(super) fn signals(&self) -> Option<Signals> {
        match self {
            Self::Providers(Some(w)) => Some(Signals::from(w)),
            _ => None,
        }
    }

    /// Jev's numbers for the turn log.
    pub(super) fn log_signals(&self) -> Value {
        match self {
            Self::Providers(Some(w)) => w.signals.clone(),
            Self::Providers(None) => Value::Null,
            Self::Legacy(w) => w.signals.clone(),
        }
    }
}
