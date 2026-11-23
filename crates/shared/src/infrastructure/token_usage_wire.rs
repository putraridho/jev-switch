use serde::{Deserialize, Serialize};

use crate::{Measurement, Usage};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MeasurementWire {
    #[default]
    Reported,
    Unavailable,
}

impl MeasurementWire {
    pub(super) fn is_reported(&self) -> bool {
        *self == Self::Reported
    }
}

impl From<Measurement> for MeasurementWire {
    fn from(value: Measurement) -> Self {
        match value {
            Measurement::Reported => Self::Reported,
            Measurement::Unavailable => Self::Unavailable,
        }
    }
}

impl From<MeasurementWire> for Measurement {
    fn from(value: MeasurementWire) -> Self {
        match value {
            MeasurementWire::Reported => Self::Reported,
            MeasurementWire::Unavailable => Self::Unavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageWire {
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    #[serde(default)]
    context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "MeasurementWire::is_reported")]
    measurement: MeasurementWire,
}

impl From<&Usage> for UsageWire {
    fn from(usage: &Usage) -> Self {
        let reported = usage.measurement == Measurement::Reported;
        Self {
            input_tokens: reported.then_some(usage.input_tokens),
            cached_input_tokens: reported.then_some(usage.cached_input_tokens),
            output_tokens: reported.then_some(usage.output_tokens),
            context_tokens: usage.context_tokens,
            measurement: usage.measurement.into(),
        }
    }
}

impl From<UsageWire> for Usage {
    fn from(wire: UsageWire) -> Self {
        Self {
            input_tokens: wire.input_tokens.unwrap_or_default(),
            cached_input_tokens: wire.cached_input_tokens.unwrap_or_default(),
            output_tokens: wire.output_tokens.unwrap_or_default(),
            context_tokens: wire.context_tokens,
            measurement: wire.measurement.into(),
            // Observations of one turn, not part of what a saved session keeps.
            cache_written_tokens: None,
            window_tokens: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Serialize, Deserialize)]
    struct SessionRow {
        usage: UsageWire,
    }

    #[test]
    fn unavailable_measurement_survives_nested_session_serialization() {
        let usage = Usage { measurement: Measurement::Unavailable, context_tokens: Some(50_000), ..Usage::default() };
        let row = SessionRow { usage: UsageWire::from(&usage) };
        let value = serde_json::to_value(row).unwrap();
        assert_eq!(value["usage"], json!({
            "input_tokens":null,"cached_input_tokens":null,"output_tokens":null,
            "context_tokens":50000,"measurement":"unavailable"
        }));
        let back: SessionRow = serde_json::from_value(value).unwrap();
        assert_eq!(Usage::from(back.usage), usage);
    }

    #[test]
    fn old_usage_rows_default_to_reported_including_known_zero() {
        for (input, cached, output) in [(0, 0, 0), (10, 4, 2)] {
            let value = json!({"usage":{"input_tokens":input,"cached_input_tokens":cached,"output_tokens":output}});
            let row: SessionRow = serde_json::from_value(value).unwrap();
            let usage = Usage::from(row.usage);
            assert_eq!(usage, Usage { input_tokens: input, cached_input_tokens: cached, output_tokens: output, ..Usage::default() });
            assert_eq!(usage.measurement, Measurement::Reported);
            let serialized = serde_json::to_value(UsageWire::from(&usage)).unwrap();
            assert!(serialized.get("measurement").is_none());
            assert_eq!(serialized["input_tokens"], input);
        }
    }

    #[test]
    fn legacy_audit_rows_still_read() {
        let value = json!({"ts_ms":1,"project":"/work","event":"turn","model":"m",
            "outcome":"answered","elapsed_ms":1,"in_tokens":0,"out_tokens":0,"prompt":"hi"});
        let wire: crate::infrastructure::audit_wire::EntryWire = serde_json::from_value(value).unwrap();
        let entry = crate::domain::audit::Entry::from(wire);
        assert!(matches!(entry.event, crate::domain::audit::Event::Turn { measurement: Measurement::Reported, in_tokens: 0, out_tokens: 0, cached_tokens: 0, context_tokens: 0, .. }));
    }
}
