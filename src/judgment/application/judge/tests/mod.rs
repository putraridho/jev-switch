//! The router against a local fake Jev; each file tests one family of judgments.

use serde_json::{json, Value};

use crate::catalog::domain::model_table::TaskType;
use crate::judgment::domain::orchestration::Orchestration;
use crate::judgment::domain::security::Security;
use crate::judgment::application::wire::choice_question;
use crate::judgment::infrastructure::typesafe_http::TypeSafe;
use crate::testkit::fake_jev::{fake_jev, stalled_jev};

use super::provider::repo_question_task;
use super::questions::*;
use super::*;

mod code_want;
mod orchestration;
mod provider;
mod questions;

/// Answers every question that was asked: `frontend` at 0.8 for the choice, and a
/// distinct probability per noul so a decoding mix-up shows.
fn answer_all(req: &Value) -> (u16, String) {
    let mut answers = serde_json::Map::new();
    for (name, q) in req["questions"].as_object().unwrap() {
        let a = if q["type"] == "choice" {
            json!({ "choice": "frontend", "confidence": 0.8, "probabilities": { "frontend": 0.8, "docs": 0.2 } })
        } else {
            let p = match name.as_str() {
                "continues" => 0.11,
                "needs_changes" => 0.22,
                "security_review" => 0.33,
                "very_hard" => 0.44,
                "is_correction" => 0.55,
                "multi_part" => 0.66,
                "sequential" => 0.11,
                _ => 0.99,
            };
            json!({ "noul": p })
        };
        answers.insert(name.clone(), a);
    }
    (200, json!({ "answers": answers }).to_string())
}

fn router_at(url: &str, timeout_ms: u64) -> Router {
    Router::new(TypeSafe::with_url("test-key", url), 1.0)
        .with_jev_timeout(std::time::Duration::from_millis(timeout_ms))
}

