//! Sending the request to Anthropic: the client's headers minus hop-by-hop ones, and
//! retries for a connection that could not be made.

use axum::body::Bytes;

use crate::application::app_state::App;
use crate::domain::haiku_adaptation::without_long_context;

/// Hop-by-hop and recomputed headers that must not be copied across the proxy.
pub(super) const SKIP_HEADERS: &[&str] =
    &["host", "content-length", "connection", "keep-alive", "transfer-encoding", "upgrade", "te", "trailer"];

/// Pauses before each retry of a connection that could not be established.
const CONNECT_RETRY_DELAYS_MS: [u64; 2] = [250, 1000];

/// Send the request, retrying only when no connection was made. Then nothing reached
/// Anthropic, so a retry can't repeat work; once bytes are sent, a failure is passed
/// to Claude Code, which has its own retry. Returns the outcome and the retries used.
pub(super) async fn send_upstream(
    app: &App,
    parts: &axum::http::request::Parts,
    path_q: &str,
    body: &Bytes,
    for_haiku: bool,
    logged: bool,
) -> (Result<reqwest::Response, reqwest::Error>, u32) {
    let mut retries = 0;
    loop {
        match upstream_request(app, parts, path_q, body.clone(), for_haiku, logged).send().await {
            Err(e) if e.is_connect() && (retries as usize) < CONNECT_RETRY_DELAYS_MS.len() => {
                tokio::time::sleep(std::time::Duration::from_millis(CONNECT_RETRY_DELAYS_MS[retries as usize])).await;
                retries += 1;
            }
            other => return (other, retries),
        }
    }
}

/// An error and its causes, e.g. "error sending request: client error (Connect):
/// dns error: ...". reqwest's own message stops at the first, which hid why 155
/// requests failed.
pub(super) fn error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        out.push_str(": ");
        out.push_str(&s.to_string());
        source = s.source();
    }
    out
}

/// The upstream request: the client's headers, minus hop-by-hop ones. A logged request
/// asks for an uncompressed response so the tap can read it; a Haiku request loses the
/// long-context beta.
fn upstream_request(app: &App, parts: &axum::http::request::Parts, path_q: &str, body: Bytes, for_haiku: bool, logged: bool) -> reqwest::RequestBuilder {
    let mut up = app.http.request(parts.method.clone(), format!("{}{}", app.upstream, path_q));
    for (k, v) in parts.headers.iter() {
        if SKIP_HEADERS.contains(&k.as_str()) || (logged && k == "accept-encoding") {
            continue;
        }
        match (for_haiku && k == "anthropic-beta", v.to_str()) {
            (true, Ok(list)) => {
                let list = without_long_context(list);
                if !list.is_empty() {
                    up = up.header(k, list);
                }
            }
            _ => up = up.header(k, v),
        }
    }
    up.body(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_chain_includes_every_cause() {
        #[derive(Debug)]
        struct Outer(std::io::Error);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "error sending request")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let e = Outer(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused"));
        assert_eq!(error_chain(&e), "error sending request: connection refused");
    }

    #[tokio::test]
    async fn refused_connection_is_a_connect_error() {
        // Nothing listens on port 1, so every attempt fails to connect.
        let client = reqwest::Client::new();
        let err = client.get("http://127.0.0.1:1/").send().await.unwrap_err();
        assert!(err.is_connect(), "a refused connection is a connect error: {err}");
        assert!(error_chain(&err).contains("onnect"), "{}", error_chain(&err));
    }
}
