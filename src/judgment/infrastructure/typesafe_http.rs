//! TypeSafe System One (Jev) client over raw HTTP.
//! We ask typed judgments — Choice and Noul — never free text.

use anyhow::{bail, Context, Result};
use crate::judgment::application::ports::{BoxFut, JevTransport};
use serde_json::{json, Value};

/// The real Jev transport: one HTTPS round trip per [`JevTransport::ask_many`].
pub struct TypeSafe {
    key: String,
    url: String,
    http: reqwest::Client,
}

const TYPESAFE_URL: &str = "https://api.typesafe.ai/v1/systemone";

/// Backstop for every Jev request. Routing applies its own shorter limit
/// (`Router::jev_timeout`); batch tools (`route`, `tune`) rely on this one.
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The Jev endpoint: `JEV_TYPESAFE_URL` (environment or env file) when it is set, for a
/// proxy or a test server; otherwise the public API.
pub fn endpoint(override_url: Option<&str>) -> String {
    override_url.map(str::trim).filter(|u| !u.is_empty()).unwrap_or(TYPESAFE_URL).to_string()
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder().timeout(HTTP_TIMEOUT).build().unwrap_or_default()
}

impl JevTransport for TypeSafe {
    fn ask_many(&self, state: Value, questions: Value) -> BoxFut<'_, Result<Value>> {
        Box::pin(self.ask(state, questions))
    }
}

impl TypeSafe {
    pub fn from_env() -> Result<Self> {
        let key = key_lookup()
            .context("Jev key not found: set TYPESAFE_API_KEY or JEV_API_KEY, or put JEV_API_KEY=... in ~/.jev-router.env")?;
        let url = endpoint(crate::shared::infrastructure::env_file::lookup("JEV_TYPESAFE_URL").as_deref());
        Ok(Self { key, url, http: http_client() })
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

}

/// The names a Jev key may be filed under, in order of preference.
const KEY_NAMES: [&str; 2] = ["TYPESAFE_API_KEY", "JEV_API_KEY"];

/// The environment wins over the key file (so a shell override works); the file is
/// what lets installed binaries work from any directory without sourcing anything.
fn key_lookup() -> Option<String> {
    KEY_NAMES.iter().find_map(|n| crate::shared::infrastructure::env_file::from_env(n)).or_else(|| crate::shared::infrastructure::env_file::from_file_any(&KEY_NAMES))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judgment::application::wire::{choice_question, noul_question};

    #[test]
    fn the_endpoint_defaults_to_the_public_api_and_can_be_overridden() {
        assert_eq!(endpoint(None), TYPESAFE_URL);
        assert_eq!(endpoint(Some("")), TYPESAFE_URL);
        assert_eq!(endpoint(Some("  ")), TYPESAFE_URL);
        assert_eq!(endpoint(Some(" http://127.0.0.1:9/v1/systemone ")), "http://127.0.0.1:9/v1/systemone");
    }

    /// A loopback server that answers one request with `answers` and returns the exact
    /// bytes of the body it received.
    async fn body_sent(call: impl std::future::Future<Output = ()>, listener: tokio::net::TcpListener) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let server = async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let body = loop {
                let n = sock.read(&mut chunk).await.unwrap();
                buf.extend_from_slice(&chunk[..n]);
                let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
                let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).unwrap().trim().parse().unwrap();
                if buf.len() >= end + 4 + len {
                    break String::from_utf8(buf[end + 4..end + 4 + len].to_vec()).unwrap();
                }
            };
            let reply = r#"{"answers":{"q":{"choice":"a","confidence":0.5},"n":{"noul":0.5}}}"#;
            let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len());
            sock.write_all(resp.as_bytes()).await.unwrap();
            body
        };
        let (body, ()) = tokio::join!(server, call);
        body
    }

    #[tokio::test]
    async fn golden_jev_request() {
        const MASKED: &str = "# masked: nothing (the key \"k\" and the loopback port are not part of the body)";
        const GOLDEN: &str = golden!("judgment/jev_request.txt");
        let sent = |call: std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>, listener| body_sent(call, listener);

        // Every optional part present: a state with nested values and two question kinds.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TypeSafe::with_url("k", &format!("http://{}/v1/systemone", listener.local_addr().unwrap()));
        let state = json!({ "request": "fix the \"login\" bug\n", "recent_context": "earlier text", "previous": { "ask": "a", "answer": "b" } });
        let questions = json!({
            "q": choice_question("pick one", json!({ "a": "the first", "b": { "what": "second", "not_for": "x", "examples": ["e1", "e2"] } })),
            "n": noul_question("is it so", json!({ "true": "yes", "false": "no" })),
        });
        let full = sent(Box::pin(async { client.ask_many(state, questions).await.unwrap(); }), listener).await;

        // Everything optional absent: one solo question, an empty state.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TypeSafe::with_url("k", &format!("http://{}/v1/systemone", listener.local_addr().unwrap()));
        let solo = sent(Box::pin(async { client.ask_many(json!({ "request": "hi" }), json!({ "q": choice_question("pick", json!({ "a": "A" })) })).await.unwrap(); }), listener).await;

        let actual = format!("=== all_parts\n{full}\n=== minimal_solo\n{solo}\n");
        let (head, body) = GOLDEN.split_once('\n').unwrap();
        assert_eq!(head, MASKED);
        assert_eq!(body, actual);
    }
}
