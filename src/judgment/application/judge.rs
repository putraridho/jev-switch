//! The router: Jev judges the *task* (not the model name), code maps it to one of
//! three tiers, and confidence gates the cheap route:
//!   simple (confident) → Haiku · reasoning (confident) → Opus · everything else → Sonnet.
//! Both cutoffs are calibrated, not guessed (see docs/guide.md).

mod code_want;
mod provider;
mod questions;
mod signals;
#[cfg(test)]
mod tests;

use anyhow::Result;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::catalog::domain::model_table::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use crate::judgment::domain::security::{is_security_sensitive, is_short_follow_up};
use crate::judgment::domain::thresholds::JEV_TIMEOUT_MS;
use crate::judgment::domain::want::Decision;
use crate::judgment::application::ports::JevTransport;
use crate::judgment::application::wire::{choice_question, decode_choice, decode_noul, noul_question, ChoiceAnswer};

/// The key a single-question call files its question under.
const SOLO: &str = "q";

pub struct Router {
    pub ts: Arc<dyn JevTransport>,
    /// Route to the cheap model only when the task is `simple` AND confidence is
    /// at least this. Do NOT guess this — read it off the `tune` sweep.
    pub cheap_threshold: f64,
    /// Route `reasoning` to the strong model only at or above this confidence;
    /// below it, the mid model. 0.0 sends all `reasoning` to strong.
    pub strong_threshold: f64,
    /// When set, a turn headed to the strong model is also asked `very_hard`, and goes
    /// to tier 3 (Fable) at or above this. `None` keeps code mode at tiers 0-2.
    pub top_threshold: Option<f64>,
    /// Orchestrated workers: a read-only `simple` task at this confidence or above gets
    /// Haiku. `None` (the default) keeps every worker on Sonnet: Haiku is
    /// uncalibrated for workers, and a wrong report from one misleads the planner.
    pub worker_cheap: Option<f64>,
    /// Longest wait for one routing judgment; past it the turn keeps its current model.
    pub jev_timeout: std::time::Duration,
}

impl Router {
    pub fn new(ts: impl JevTransport + 'static, cheap_threshold: f64) -> Self {
        let ms = crate::shared::application::host::env("JEV_TIMEOUT_MS").and_then(|v| v.parse().ok()).unwrap_or(JEV_TIMEOUT_MS);
        Self {
            ts: Arc::new(ts),
            cheap_threshold,
            strong_threshold: 0.0,
            top_threshold: None,
            worker_cheap: None,
            jev_timeout: std::time::Duration::from_millis(ms),
        }
    }

    /// One Choice judgment over `state`. `criteria` is a JSON object mapping each
    /// option name to its description (string, or an object with `what`/`not_for`/
    /// `examples` when options risk confusion).
    async fn choice(&self, state: Value, instructions: &str, criteria: Value) -> Result<ChoiceAnswer> {
        let answers = self.ask_one(state, choice_question(instructions, criteria)).await?;
        decode_choice(&answers, SOLO)
    }

    /// One Noul (yes/no) judgment; returns P(true) in 0..=1.
    async fn noul(&self, state: Value, instructions: &str, criteria: Value) -> Result<f64> {
        self.noul_question(state, noul_question(instructions, criteria)).await
    }

    /// One prebuilt Noul question (see [`noul_question`]), asked alone. Lets a
    /// single-call method send exactly the question definition the batch uses.
    async fn noul_question(&self, state: Value, question: Value) -> Result<f64> {
        let answers = self.ask_one(state, question).await?;
        decode_noul(&answers, SOLO)
    }

    /// One already-built question asked alone, filed under [`SOLO`].
    async fn ask_one(&self, state: Value, question: Value) -> Result<Value> {
        self.ts.ask_many(state, json!({ SOLO: question })).await
    }

    pub fn with_jev_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.jev_timeout = timeout;
        self
    }

    pub fn with_worker_cheap(mut self, worker_cheap: Option<f64>) -> Self {
        self.worker_cheap = worker_cheap;
        self
    }


    pub fn with_top_threshold(mut self, top_threshold: Option<f64>) -> Self {
        self.top_threshold = top_threshold;
        self
    }

    pub fn with_strong_threshold(mut self, strong_threshold: f64) -> Self {
        self.strong_threshold = strong_threshold;
        self
    }

    /// The single routing judgment, shared by `route` (production) and `tune`
    /// (calibration) so both evaluate the exact same question.
    pub async fn classify(&self, request: &str, recent_context: &str) -> Result<ChoiceAnswer> {
        self.choice(
                json!({ "request": request, "recent_context": recent_context }),
                "What does this coding request need from the model?",
                json!({
                    "simple": {
                        "what": "a direct lookup, formatting, a short edit, a factual answer, or a small self-contained snippet: a one-liner, regex, command, config file, or a short standard function (under ~20 lines) with no multi-step reasoning",
                        "not_for": "anything needing planning, debugging across files, or tradeoff analysis; or any security-sensitive code (auth, crypto, signatures, passwords, tokens, input sanitization, injection)",
                        "examples": ["rename a variable", "what does this flag do", "format this JSON", "write a one-line regex", "check if a string is a palindrome", "write a Dockerfile for a Node app", "find the largest file in a directory"]
                    },
                    "standard": {
                        "what": "everyday coding that needs more than a short snippet but stays within one file or component: a module, class, script, middleware, CLI, tests, a small refactor, or debugging a specific error in given code",
                        "not_for": "one-liners, lookups, or short standard functions (simple), or work spanning many files or systems, architecture, or tradeoffs (reasoning)",
                        "examples": ["write a CLI that parses a CSV and prints a column average", "add pagination to this endpoint", "why does this async function return undefined", "write unit tests for this function"]
                    },
                    "reasoning": {
                        "what": "multi-step problem solving, system design, planning, debugging across files or systems, math or logic, or tradeoff analysis",
                        "examples": ["why does the upload hang on large files", "design the retry and backoff strategy", "refactor this module to remove the cycle"]
                    },
                    "unclear": {
                        "what": "ambiguous, mixed, or under-specified; not clearly simple, standard, or reasoning"
                    }
                }),
            )
            .await
    }

    /// A-priori routing from the request text (+ recent context), for a request
    /// with no prior turn to follow.
    pub async fn route(&self, request: &str, recent_context: &str) -> Result<Decision> {
        self.route_turn(request, recent_context, None).await
    }

    /// Routing for a turn in a conversation. `prev_model` is the model that
    /// answered the previous turn, if any. Precedence:
    /// 1. security-sensitive requests always go strong (hard rule, not Jev's call);
    /// 2. a short follow-up stays on the previous model unless Jev is confident
    ///    about a tier (confidently simple or confidently reasoning);
    /// 3. otherwise Jev's judgment: confidently simple → cheap; confidently
    ///    reasoning → strong; everything else (unsure simple/reasoning, standard,
    ///    unclear) → mid.
    pub async fn route_turn(
        &self,
        request: &str,
        recent_context: &str,
        prev_model: Option<&'static str>,
    ) -> Result<Decision> {
        let ans = self.classify(request, recent_context).await?;
        let go_cheap = ans.choice == "simple" && ans.confidence >= self.cheap_threshold;
        let go_strong = ans.choice == "reasoning" && ans.confidence >= self.strong_threshold;
        let (model, reason) = if is_security_sensitive(request) {
            (MODEL_STRONG, "security-sensitive \u{2192} strong")
        } else if let Some(prev) = prev_model.filter(|_| {
            !go_cheap && !go_strong && is_short_follow_up(request)
        }) {
            (prev, "short follow-up \u{2192} same model as last turn")
        } else if go_cheap {
            (MODEL_CHEAP, "confidently simple")
        } else if go_strong {
            (MODEL_STRONG, "confidently reasoning \u{2192} strong")
        } else {
            (MODEL_MID, "standard / unclear / low-confidence \u{2192} mid")
        };
        Ok(Decision { model, kind: ans.choice, confidence: ans.confidence, reason })
    }

    /// Verify variant: judge whether a cheap answer actually resolved the request.
    /// Returns true when adequate. Gate its probability on a threshold you also
    /// calibrate; this tracks outcome quality far more stably than a-priori
    /// difficulty, so it is the recommended escalation signal.
    pub async fn answer_is_adequate(
        &self,
        request: &str,
        answer: &str,
        threshold: f64,
    ) -> Result<(bool, f64)> {
        let p = self
            .noul(
                json!({ "request": request, "answer": answer }),
                "Does the answer fully and correctly address the request?",
                json!({
                    "true": "complete, correct, and directly responsive",
                    "false": "incomplete, wrong, evasive, or only partially addresses it"
                }),
            )
            .await?;
        Ok((p >= threshold, p))
    }
}
