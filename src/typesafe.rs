//! TypeSafe System One (Jev) client over raw HTTP.
//! We ask typed judgments — Choice and Noul — never free text.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct ChoiceAnswer {
    pub choice: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub probabilities: HashMap<String, f64>,
}

pub struct TypeSafe {
    key: String,
    http: reqwest::Client,
}

impl TypeSafe {
    pub fn from_env() -> Result<Self> {
        let key = std::env::var("TYPESAFE_API_KEY")
            .or_else(|_| std::env::var("JEV_API_KEY"))
            .ok()
            .or_else(key_from_env_file)
            .context("Jev key not found: set TYPESAFE_API_KEY or JEV_API_KEY, or put JEV_API_KEY=... in ~/.jev-router.env")?;
        Ok(Self { key, http: reqwest::Client::new() })
    }

    async fn ask(&self, state: Value, questions: Value) -> Result<Value> {
        let req = json!({ "state": state, "model": "jev-latest", "questions": questions });
        let resp = self
            .http
            .post("https://api.typesafe.ai/v1/systemone")
            .header("authorization", format!("Bearer {}", self.key))
            .json(&req)
            .send()
            .await
            .context("typesafe request failed")?;
        let status = resp.status();
        let body = resp.text().await.context("read typesafe body")?;
        if !status.is_success() {
            bail!("typesafe {}: {}", status, body);
        }
        let v: Value = serde_json::from_str(&body).context("parse typesafe response")?;
        Ok(v.get("answers").cloned().unwrap_or(Value::Null))
    }

    /// One Choice judgment over `state`. `criteria` is a JSON object mapping each
    /// option name to its description (string, or an object with `what`/`not_for`/
    /// `examples` when options risk confusion).
    pub async fn choice(
        &self,
        state: Value,
        instructions: &str,
        criteria: Value,
    ) -> Result<ChoiceAnswer> {
        let answers = self
            .ask(
                state,
                json!({ "q": { "type": "choice", "instructions": instructions, "criteria": criteria } }),
            )
            .await?;
        let a = answers.get("q").cloned().context("missing choice answer")?;
        serde_json::from_value(a).context("decode choice answer")
    }

    /// One Noul (yes/no) judgment; returns P(true) in 0..=1.
    pub async fn noul(
        &self,
        state: Value,
        instructions: &str,
        criteria: Value,
    ) -> Result<f64> {
        let answers = self
            .ask(
                state,
                json!({ "q": { "type": "noul", "instructions": instructions, "criteria": criteria } }),
            )
            .await?;
        answers
            .get("q")
            .and_then(|q| q.get("noul"))
            .and_then(|n| n.as_f64())
            .context("missing noul value")
    }
}

/// Read the key from `$JEV_ROUTER_ENV` or `~/.jev-router.env` (KEY=VALUE lines),
/// so installed binaries work from any directory without sourcing anything.
fn key_from_env_file() -> Option<String> {
    let path = std::env::var("JEV_ROUTER_ENV")
        .ok()
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.jev-router.env")))?;
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|line| {
        let line = line.trim().trim_start_matches("export ").trim();
        let (k, v) = line.split_once('=')?;
        matches!(k.trim(), "TYPESAFE_API_KEY" | "JEV_API_KEY")
            .then(|| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
            .filter(|v| !v.is_empty())
    })
}
