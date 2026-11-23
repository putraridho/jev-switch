//! What Jev said about one turn.

use serde_json::Value;

use jev_judgment::domain::want::{CodeWant, ProviderWant};
use jev_catalog::domain::model_table::TaskType;
use jev_judgment::domain::orchestration::Orchestration;
use jev_judgment::domain::security::{security_level, Security};
use jev_routing::domain::decide::Signals;
/// What Jev (or the legacy tier judgment) said about one turn.
pub enum Judged {
    /// Provider routing; `None` when Jev was unavailable.
    Providers(Option<ProviderWant>),
    /// Routing off: the old Claude-only tier judgment.
    Legacy(CodeWant),
}

impl Judged {
    pub(super) fn signals(&self, prompt: &str) -> Option<Signals> {
        let security = security_level(prompt);
        let mut signals = match self {
            Self::Providers(Some(w)) => Some(Signals::from(w)),
            Self::Providers(None) if security == Security::Strong => Some(Signals {
                task: TaskType::Unclear,
                task_confidence: 0.0,
                very_hard_p: 0.0,
                security_p: 0.0,
                edit_p: 0.0,
                continues_p: 1.0,
                correction_p: None,
                orchestration: Orchestration::default(),
                security_keywords: security,
            }),
            _ => None,
        };
        if let Some(signals) = &mut signals {
            signals.security_keywords = signals.security_keywords.max(security);
        }
        signals
    }

    /// Jev's numbers for the turn log.
    pub(super) fn log_signals(&self) -> Value {
        match self {
            Self::Providers(Some(w)) => w.signals.to_json(),
            Self::Providers(None) => Value::Null,
            Self::Legacy(w) => w.signals.to_json(),
        }
    }
}
