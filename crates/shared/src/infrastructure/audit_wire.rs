//! JSON representation of the shared audit entries.

use serde::{Deserialize, Serialize};

use crate::domain::audit::{Entry, Event, Outcome};
use crate::Measurement;
use super::token_usage_wire::MeasurementWire;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OutcomeWire {
    Answered,
    NoRoute,
    Failed,
    Cancelled,
}

impl From<Outcome> for OutcomeWire {
    fn from(value: Outcome) -> Self {
        match value {
            Outcome::Answered => Self::Answered,
            Outcome::NoRoute => Self::NoRoute,
            Outcome::Failed => Self::Failed,
            Outcome::Cancelled => Self::Cancelled,
        }
    }
}

impl From<OutcomeWire> for Outcome {
    fn from(value: OutcomeWire) -> Self {
        match value {
            OutcomeWire::Answered => Self::Answered,
            OutcomeWire::NoRoute => Self::NoRoute,
            OutcomeWire::Failed => Self::Failed,
            OutcomeWire::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum EventWire {
    Turn {
        model: String,
        outcome: OutcomeWire,
        elapsed_ms: u64,
        in_tokens: Option<u64>,
        out_tokens: Option<u64>,
        prompt: String,
        #[serde(default)]
        cached_tokens: Option<u64>,
        #[serde(default)]
        context_tokens: u64,
        #[serde(default)]
        tool_calls: u32,
        #[serde(default, skip_serializing_if = "MeasurementWire::is_reported")]
        measurement: MeasurementWire,
    },
    Export { path: String },
    Profile { name: String },
}

impl From<Event> for EventWire {
    fn from(value: Event) -> Self {
        match value {
            Event::Turn { model, outcome, measurement, elapsed_ms, in_tokens, out_tokens, prompt, cached_tokens, context_tokens, tool_calls } => {
                let reported = measurement == Measurement::Reported;
                Self::Turn {
                    model,
                    outcome: outcome.into(),
                    elapsed_ms,
                    in_tokens: reported.then_some(in_tokens),
                    out_tokens: reported.then_some(out_tokens),
                    prompt,
                    cached_tokens: reported.then_some(cached_tokens),
                    context_tokens,
                    tool_calls,
                    measurement: measurement.into(),
                }
            }
            Event::Export { path } => Self::Export { path },
            Event::Profile { name } => Self::Profile { name },
        }
    }
}

impl From<EventWire> for Event {
    fn from(value: EventWire) -> Self {
        match value {
            EventWire::Turn { model, outcome, measurement, elapsed_ms, in_tokens, out_tokens, prompt, cached_tokens, context_tokens, tool_calls } =>
                Self::Turn { model, outcome: outcome.into(), measurement: measurement.into(), elapsed_ms, in_tokens: in_tokens.unwrap_or_default(), out_tokens: out_tokens.unwrap_or_default(), prompt, cached_tokens: cached_tokens.unwrap_or_default(), context_tokens, tool_calls },
            EventWire::Export { path } => Self::Export { path },
            EventWire::Profile { name } => Self::Profile { name },
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct EntryWire {
    ts_ms: u64,
    project: String,
    #[serde(flatten)]
    event: EventWire,
}

impl From<&Entry> for EntryWire {
    fn from(value: &Entry) -> Self {
        Self { ts_ms: value.ts_ms, project: value.project.clone(), event: value.event.clone().into() }
    }
}

impl From<EntryWire> for Entry {
    fn from(value: EntryWire) -> Self {
        Self { ts_ms: value.ts_ms, project: value.project, event: value.event.into() }
    }
}
