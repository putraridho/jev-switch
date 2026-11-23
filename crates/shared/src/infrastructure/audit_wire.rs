//! JSON representation of the shared audit entries.

use serde::{Deserialize, Serialize};

use crate::domain::acceptance::{Acceptance, Quality, Rejection, Rework};
use crate::domain::audit::{Entry, Event, Outcome, Verdict};
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
#[serde(rename_all = "snake_case")]
enum VerdictWire {
    Passed,
    Failed,
}

impl From<Verdict> for VerdictWire {
    fn from(value: Verdict) -> Self {
        match value {
            Verdict::Passed => Self::Passed,
            Verdict::Failed => Self::Failed,
        }
    }
}

impl From<VerdictWire> for Verdict {
    fn from(value: VerdictWire) -> Self {
        match value {
            VerdictWire::Passed => Self::Passed,
            VerdictWire::Failed => Self::Failed,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AcceptanceWire {
    Accepted,
    RejectedCheckFailed,
    RejectedReverted,
    #[default]
    Unverified,
}

impl From<Acceptance> for AcceptanceWire {
    fn from(value: Acceptance) -> Self {
        match value {
            Acceptance::Accepted => Self::Accepted,
            Acceptance::Rejected(Rejection::VerificationFailed) => Self::RejectedCheckFailed,
            Acceptance::Rejected(Rejection::Reverted) => Self::RejectedReverted,
            Acceptance::Unverified => Self::Unverified,
        }
    }
}

impl From<AcceptanceWire> for Acceptance {
    fn from(value: AcceptanceWire) -> Self {
        match value {
            AcceptanceWire::Accepted => Self::Accepted,
            AcceptanceWire::RejectedCheckFailed => Self::Rejected(Rejection::VerificationFailed),
            AcceptanceWire::RejectedReverted => Self::Rejected(Rejection::Reverted),
            AcceptanceWire::Unverified => Self::Unverified,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReworkWire {
    #[default]
    None,
    AfterFailedCheck,
    AfterRevert,
}

impl From<Rework> for ReworkWire {
    fn from(value: Rework) -> Self {
        match value {
            Rework::None => Self::None,
            Rework::AfterFailedCheck => Self::AfterFailedCheck,
            Rework::AfterRevert => Self::AfterRevert,
        }
    }
}

impl From<ReworkWire> for Rework {
    fn from(value: ReworkWire) -> Self {
        match value {
            ReworkWire::None => Self::None,
            ReworkWire::AfterFailedCheck => Self::AfterFailedCheck,
            ReworkWire::AfterRevert => Self::AfterRevert,
        }
    }
}

/// The quality fields of a turn line; every one reads as its empty value when missing.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Eq)]
struct QualityWire {
    #[serde(default)]
    task: String,
    #[serde(default)]
    acceptance: AcceptanceWire,
    #[serde(default)]
    verify_ms: u64,
    #[serde(default)]
    retries: u32,
    #[serde(default)]
    rework: ReworkWire,
    #[serde(default)]
    spent_tokens: u64,
}

impl QualityWire {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl From<Quality> for QualityWire {
    fn from(q: Quality) -> Self {
        Self { task: q.task, acceptance: q.acceptance.into(), verify_ms: q.verify_ms, retries: q.retries, rework: q.rework.into(), spent_tokens: q.spent_tokens }
    }
}

impl From<QualityWire> for Quality {
    fn from(q: QualityWire) -> Self {
        Self { task: q.task, acceptance: q.acceptance.into(), verify_ms: q.verify_ms, retries: q.retries, rework: q.rework.into(), spent_tokens: q.spent_tokens }
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        verdict: Option<VerdictWire>,
        #[serde(default, skip_serializing_if = "QualityWire::is_empty")]
        quality: QualityWire,
    },
    Export { path: String },
    Profile { name: String },
}

impl From<Event> for EventWire {
    fn from(value: Event) -> Self {
        match value {
            Event::Turn { model, outcome, measurement, elapsed_ms, in_tokens, out_tokens, prompt, cached_tokens, context_tokens, tool_calls, verdict, quality } => {
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
                    verdict: verdict.map(Into::into),
                    quality: quality.into(),
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
            EventWire::Turn { model, outcome, measurement, elapsed_ms, in_tokens, out_tokens, prompt, cached_tokens, context_tokens, tool_calls, verdict, quality } =>
                Self::Turn { model, outcome: outcome.into(), measurement: measurement.into(), elapsed_ms, in_tokens: in_tokens.unwrap_or_default(), out_tokens: out_tokens.unwrap_or_default(), prompt, cached_tokens: cached_tokens.unwrap_or_default(), context_tokens, tool_calls, verdict: verdict.map(Into::into), quality: quality.into() },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::acceptance::Quality;

    fn turn(quality: Quality) -> Entry {
        Entry {
            ts_ms: 1,
            project: "/p".into(),
            event: Event::Turn { model: "m".into(), outcome: Outcome::Answered, measurement: Measurement::Reported, elapsed_ms: 5, in_tokens: 1, out_tokens: 1, prompt: "x".into(), cached_tokens: 0, context_tokens: 0, tool_calls: 0, verdict: None, quality },
        }
    }

    #[test]
    fn quality_survives_a_round_trip() {
        let q = Quality { task: "feature".into(), acceptance: Acceptance::Rejected(Rejection::Reverted), verify_ms: 1200, retries: 2, rework: Rework::AfterRevert, spent_tokens: 4321 };
        let line = serde_json::to_string(&EntryWire::from(&turn(q.clone()))).unwrap();
        let Event::Turn { quality, .. } = Entry::from(serde_json::from_str::<EntryWire>(&line).unwrap()).event else { panic!() };
        assert_eq!(quality, q);
    }

    #[test]
    fn a_line_without_quality_reads_as_unverified_and_an_empty_one_is_not_written() {
        let old = r#"{"ts_ms":1,"project":"/p","event":"turn","model":"m","outcome":"answered","elapsed_ms":5,"in_tokens":1,"out_tokens":1,"prompt":"x"}"#;
        let Event::Turn { quality, .. } = Entry::from(serde_json::from_str::<EntryWire>(old).unwrap()).event else { panic!() };
        assert_eq!(quality, Quality::default());
        assert_eq!(quality.acceptance, Acceptance::Unverified);
        let line = serde_json::to_string(&EntryWire::from(&turn(Quality::default()))).unwrap();
        assert!(!line.contains("quality"), "{line}");
        let partial = old.replace(r#""prompt":"x""#, r#""prompt":"x","quality":{"acceptance":"accepted"}"#);
        let Event::Turn { quality, .. } = Entry::from(serde_json::from_str::<EntryWire>(&partial).unwrap()).event else { panic!() };
        assert_eq!((quality.acceptance, quality.spent_tokens), (Acceptance::Accepted, 0));
    }
}
