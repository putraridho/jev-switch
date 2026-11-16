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
    url: String,
    http: reqwest::Client,
}

const TYPESAFE_URL: &str = "https://api.typesafe.ai/v1/systemone";

/// Backstop for every Jev request. Routing applies its own shorter limit
/// (`Router::jev_timeout`); batch tools (`route`, `tune`) rely on this one.
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

fn http_client() -> reqwest::Client {
    reqwest::Client::builder().timeout(HTTP_TIMEOUT).build().unwrap_or_default()
}

impl TypeSafe {
    pub fn from_env() -> Result<Self> {
        let key = key_lookup()
            .context("Jev key not found: set TYPESAFE_API_KEY or JEV_API_KEY, or put JEV_API_KEY=... in ~/.jev-router.env")?;
        Ok(Self { key, url: TYPESAFE_URL.into(), http: http_client() })
    }

    /// A client for a different endpoint (tests, proxies).
    pub fn with_url(key: &str, url: &str) -> Self {
        Self { key: key.into(), url: url.into(), http: http_client() }
    }

    async fn ask(&self, state: Value, questions: Value) -> Result<Value> {
        let req = json!({ "state": state, "model": "jev-latest", "questions": questions });
        let resp = self
            .http
            .post(&self.url)
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

/// The names a Jev key may be filed under, in order of preference.
const KEY_NAMES: [&str; 2] = ["TYPESAFE_API_KEY", "JEV_API_KEY"];

/// The environment wins over the key file (so a shell override works); the file is
/// what lets installed binaries work from any directory without sourcing anything.
fn key_lookup() -> Option<String> {
    KEY_NAMES.iter().find_map(|n| crate::envfile::from_env(n)).or_else(|| crate::envfile::from_file_any(&KEY_NAMES))
}
