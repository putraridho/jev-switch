//! The one HTTP client (DESIGN 8.7): the only module allowed to use `reqwest`, and the only
//! place a secret is exposed. The types are frozen; T05 writes the bodies.

use jev_usage_core::secret::Secret;
use jev_usage_core::usage::Unavailable;
use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::redirect::Policy;
use reqwest::{Client, Error, StatusCode};
use std::borrow::Cow;
use std::time::Duration;
use zeroize::Zeroizing;

/// The production DeepSeek balance endpoint (DESIGN 6.4, 8.7).
const DEEPSEEK_BALANCE_URL: &str = "https://api.deepseek.com/user/balance";
/// Fixed generic user agent (DESIGN 8.7).
const USER_AGENT_VALUE: &str = "jev-dashboard";
/// Maximum response body accepted (64 KiB, DESIGN 8.7).
const MAX_BODY_BYTES: usize = 64 * 1024;
/// Maximum response body as `u64` for `content_length()` pre-check.
const MAX_BODY_BYTES_U64: u64 = 64 * 1024;
/// Connect timeout (DESIGN 8.7).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Total request timeout (DESIGN 8.7).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The places the app may call. Each maps to a constant HTTPS URL.
#[derive(Copy, Clone, Debug)]
pub enum Endpoint {
    /// `GET https://api.deepseek.com/user/balance`.
    DeepSeekBalance,
}

impl Endpoint {
    const fn url(self) -> &'static str {
        match self {
            Self::DeepSeekBalance => DEEPSEEK_BALANCE_URL,
        }
    }

    #[cfg(feature = "test-support")]
    const fn path(self) -> &'static str {
        match self {
            Self::DeepSeekBalance => "/user/balance",
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
struct ClientConfig {
    https_only: bool,
    no_redirects: bool,
    no_proxy: bool,
    connect_timeout: Duration,
    timeout: Duration,
    user_agent: &'static str,
    max_body_bytes: usize,
}

const fn production_config() -> ClientConfig {
    ClientConfig {
        https_only: true,
        no_redirects: true,
        no_proxy: true,
        connect_timeout: CONNECT_TIMEOUT,
        timeout: REQUEST_TIMEOUT,
        user_agent: USER_AGENT_VALUE,
        max_body_bytes: MAX_BODY_BYTES,
    }
}

fn build_client(config: ClientConfig) -> Result<Client, Unavailable> {
    let mut builder = Client::builder()
        .use_rustls_tls()
        .https_only(config.https_only)
        .user_agent(config.user_agent)
        .connect_timeout(config.connect_timeout)
        .timeout(config.timeout);
    if config.no_redirects {
        builder = builder.redirect(Policy::none());
    }
    if config.no_proxy {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|_| Unavailable::Internal)
}

fn build_bearer_header(bearer: &Secret<String>) -> Result<HeaderValue, Unavailable> {
    let raw = bearer.expose_secret();
    if raw.trim().is_empty() {
        return Err(Unavailable::Unauthorized);
    }
    let prefix = "Bearer ";
    let capacity = prefix.len().saturating_add(raw.len());
    let mut header_text = Zeroizing::new(String::with_capacity(capacity));
    header_text.push_str(prefix);
    header_text.push_str(raw);
    let mut value = HeaderValue::from_str(&header_text).map_err(|_| Unavailable::Unauthorized)?;
    value.set_sensitive(true);
    Ok(value)
}

fn map_status(status: StatusCode) -> Unavailable {
    match status.as_u16() {
        401 | 403 => Unavailable::Unauthorized,
        429 => Unavailable::RateLimited,
        500..=599 => Unavailable::ServerError,
        _ => Unavailable::Protocol,
    }
}

fn classify_send_error(err: Error) -> Unavailable {
    let reason = if err.is_timeout() {
        Unavailable::Timeout
    } else if err.is_connect() {
        Unavailable::Offline
    } else if let Some(status) = err.status() {
        map_status(status)
    } else if err.is_body() || err.is_decode() {
        Unavailable::Protocol
    } else if err.is_builder() {
        Unavailable::Internal
    } else {
        Unavailable::Offline
    };
    drop(err);
    reason
}

fn classify_body_error(err: Error) -> Unavailable {
    let reason = if err.is_timeout() {
        Unavailable::Timeout
    } else {
        Unavailable::Protocol
    };
    drop(err);
    reason
}

/// rustls, HTTPS only, no redirects, no proxy, no cookies, bounded time and body.
#[derive(Debug)]
pub struct HttpClient {
    client: Client,
    #[cfg(feature = "test-support")]
    base: Option<String>,
}

impl HttpClient {
    /// The production client with the settings of DESIGN 8.7.
    pub fn new() -> Result<Self, Unavailable> {
        Ok(Self {
            client: build_client(production_config())?,
            #[cfg(feature = "test-support")]
            base: None,
        })
    }

    /// A client that calls a local stub server at `base` instead (feature `test-support`;
    /// plain HTTP is allowed only here).
    #[cfg(feature = "test-support")]
    pub fn with_base(base: &str) -> Result<Self, Unavailable> {
        let mut config = production_config();
        config.https_only = false;
        Ok(Self {
            client: build_client(config)?,
            base: Some(base.trim_end_matches('/').to_owned()),
        })
    }

    /// One GET with the bearer key; the body, at most 64 KiB.
    pub async fn get_json(
        &self,
        endpoint: Endpoint,
        bearer: &Secret<String>,
    ) -> Result<Vec<u8>, Unavailable> {
        let url = self.resolve_url(endpoint);
        let auth_header = build_bearer_header(bearer)?;
        let mut response = self
            .client
            .get(url.as_ref())
            .header(AUTHORIZATION, auth_header)
            .send()
            .await
            .map_err(classify_send_error)?;

        if response.status() != StatusCode::OK {
            return Err(map_status(response.status()));
        }

        if response
            .content_length()
            .is_some_and(|len| len > MAX_BODY_BYTES_U64)
        {
            return Err(Unavailable::Protocol);
        }

        let initial_cap = response
            .content_length()
            .and_then(|len| usize::try_from(len).ok())
            .map_or(0, |len| len.min(MAX_BODY_BYTES));
        let mut body = Vec::with_capacity(initial_cap);
        while let Some(chunk) = response.chunk().await.map_err(classify_body_error)? {
            let next_len = body
                .len()
                .checked_add(chunk.len())
                .ok_or(Unavailable::Protocol)?;
            if next_len > MAX_BODY_BYTES {
                return Err(Unavailable::Protocol);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    fn resolve_url(&self, endpoint: Endpoint) -> Cow<'static, str> {
        #[cfg(feature = "test-support")]
        if let Some(base) = &self.base {
            return Cow::Owned(format!("{base}{}", endpoint.path()));
        }
        Cow::Borrowed(endpoint.url())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    #[test]
    fn production_config_matches_design_8_7() {
        let cfg = production_config();
        assert!(cfg.https_only);
        assert!(cfg.no_redirects);
        assert!(cfg.no_proxy);
        assert_eq!(cfg.connect_timeout, Duration::from_secs(5));
        assert_eq!(cfg.timeout, Duration::from_secs(10));
        assert_eq!(cfg.user_agent, "jev-dashboard");
        assert_eq!(cfg.max_body_bytes, 64 * 1024);
        assert!(build_client(cfg).is_ok());
    }

    #[test]
    fn production_endpoint_is_the_constant_deepseek_https_url() {
        assert_eq!(
            Endpoint::DeepSeekBalance.url(),
            "https://api.deepseek.com/user/balance"
        );
    }

    #[test]
    fn authorization_header_is_marked_sensitive_and_rejects_blank() {
        let valid = Secret::new("JEVCANARY_KEY_test".to_owned());
        let header = build_bearer_header(&valid).unwrap();
        assert!(header.is_sensitive());
        assert!(!format!("{header:?}").contains("JEVCANARY_KEY_test"));

        let empty = Secret::new("   ".to_owned());
        assert_eq!(build_bearer_header(&empty), Err(Unavailable::Unauthorized));

        let newline = Secret::new("bad\nkey".to_owned());
        assert_eq!(
            build_bearer_header(&newline),
            Err(Unavailable::Unauthorized)
        );
    }

    #[tokio::test]
    async fn production_client_rejects_plain_http_without_connecting() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_bg = Arc::clone(&hits);
        let server = tokio::spawn(async move {
            while let Ok((_stream, _)) = listener.accept().await {
                hits_bg.fetch_add(1, Ordering::SeqCst);
            }
        });

        let client = HttpClient::new().unwrap();
        let err = client
            .client
            .get(format!("http://{addr}/user/balance"))
            .send()
            .await
            .unwrap_err();
        assert_eq!(classify_send_error(err), Unavailable::Internal);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        server.abort();
    }
}
