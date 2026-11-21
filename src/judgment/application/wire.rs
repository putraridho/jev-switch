//! What the router says to Jev and how it reads the answers: the question shapes and
//! the decoders. Pure, so question text lives in one place and tests compare it
//! without a network.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::judgment::domain::want::JevSignals;

impl From<Value> for JevSignals {
    fn from(value: Value) -> Self {
        Self::new(value.to_string(), value["choice"].as_str().map(str::to_owned), value["confidence"].as_f64(), value["edit_p"].as_f64())
    }
}

impl JevSignals {
    pub fn to_json(&self) -> Value {
        serde_json::from_str(self.raw()).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Deserialize)]
pub struct ChoiceAnswer {
    pub choice: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub probabilities: HashMap<String, f64>,
}

/// A Choice question for [`super::ports::JevTransport::ask_many`]. Pure, so question text lives in one
/// place and tests can compare it without a network.
pub fn choice_question(instructions: &str, criteria: Value) -> Value {
    json!({ "type": "choice", "instructions": instructions, "criteria": criteria })
}

/// A Noul (yes/no) question for [`super::ports::JevTransport::ask_many`].
pub fn noul_question(instructions: &str, criteria: Value) -> Value {
    json!({ "type": "noul", "instructions": instructions, "criteria": criteria })
}

/// Read the Choice answer filed under `key` in an `answers` object.
pub fn decode_choice(answers: &Value, key: &str) -> Result<ChoiceAnswer> {
    let a = answers.get(key).cloned().with_context(|| format!("missing choice answer `{key}`"))?;
    serde_json::from_value(a).with_context(|| format!("decode choice answer `{key}`"))
}

/// Read the Noul answer filed under `key`: P(true) in 0..=1.
pub fn decode_noul(answers: &Value, key: &str) -> Result<f64> {
    answers
        .get(key)
        .and_then(|q| q.get("noul"))
        .and_then(|n| n.as_f64())
        .with_context(|| format!("missing noul value `{key}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoders_read_each_answer_by_its_key() {
        let answers = json!({
            "a": { "choice": "x", "confidence": 0.9, "probabilities": { "x": 0.9 } },
            "b": { "noul": 0.25 }
        });
        let c = decode_choice(&answers, "a").unwrap();
        assert_eq!((c.choice.as_str(), c.confidence), ("x", 0.9));
        assert_eq!(decode_noul(&answers, "b").unwrap(), 0.25);
        assert!(decode_choice(&answers, "missing").is_err());
        assert!(decode_noul(&answers, "a").is_err(), "a choice is not a noul");
    }

    #[test]
    fn question_builders_keep_the_wire_shape() {
        assert_eq!(
            noul_question("why", json!({ "true": "t", "false": "f" })),
            json!({ "type": "noul", "instructions": "why", "criteria": { "true": "t", "false": "f" } })
        );
        assert_eq!(choice_question("pick", json!({ "a": "A" }))["type"], "choice");
    }
}
