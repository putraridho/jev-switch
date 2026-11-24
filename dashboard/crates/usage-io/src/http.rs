//! The one HTTP client (DESIGN 8.7): the only module allowed to use `reqwest`, and the only
//! place a secret is exposed. The types are frozen; T05 writes the bodies.

use jev_usage_core::secret::Secret;
use jev_usage_core::usage::Unavailable;

/// The places the app may call. Each maps to a constant HTTPS URL.
#[derive(Copy, Clone, Debug)]
pub enum Endpoint {
    /// `GET https://api.deepseek.com/user/balance`.
    DeepSeekBalance,
}

/// rustls, HTTPS only, no redirects, no proxy, no cookies, bounded time and body.
#[derive(Debug)]
pub struct HttpClient {
    _private: (),
}

impl HttpClient {
    /// The production client with the settings of DESIGN 8.7.
    pub fn new() -> Result<Self, Unavailable> {
        Ok(Self { _private: () })
    }

    /// A client that calls a local stub server at `base` instead (feature `test-support`;
    /// plain HTTP is allowed only here).
    #[cfg(feature = "test-support")]
    pub fn with_base(base: &str) -> Result<Self, Unavailable> {
        let _ = base;
        Ok(Self { _private: () })
    }

    /// One GET with the bearer key; the body, at most 64 KiB.
    pub async fn get_json(
        &self,
        endpoint: Endpoint,
        bearer: &Secret<String>,
    ) -> Result<Vec<u8>, Unavailable> {
        let _ = (endpoint, bearer);
        Err(Unavailable::Internal)
    }
}
