//! The DeepSeek balance as a source (DESIGN 6.4). The type and the descriptor are fixed
//! here; T05 writes the body.

use crate::http::HttpClient;
use crate::secrets::KeySources;
use jev_usage_core::port::{
    Cost, FetchFuture, SourceDescriptor, SourceError, SourceId, Trigger, UsageSource,
};
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::time::Clock;
use jev_usage_core::usage::Unavailable;
use std::sync::Arc;
use std::time::Duration;

/// Reads the balance with a key that lives for one fetch.
pub struct DeepSeekBalanceSource {
    _http: Arc<HttpClient>,
    _clock: Arc<dyn Clock>,
    _keys: Arc<dyn Fn() -> KeySources + Send + Sync>,
}

impl DeepSeekBalanceSource {
    /// A source that asks `keys` for the key at each fetch.
    pub fn new(
        http: Arc<HttpClient>,
        clock: Arc<dyn Clock>,
        keys: Arc<dyn Fn() -> KeySources + Send + Sync>,
    ) -> Self {
        Self {
            _http: http,
            _clock: clock,
            _keys: keys,
        }
    }
}

impl UsageSource for DeepSeekBalanceSource {
    fn descriptor(&self) -> SourceDescriptor {
        SourceDescriptor {
            id: SourceId::DeepSeekBalance,
            provider: ProviderKey::DeepSeek,
            trigger: Trigger::Interval {
                visible: Duration::from_mins(10),
                hidden: Duration::from_mins(30),
            },
            cost: Cost::Free,
            timeout: Duration::from_secs(10),
            // 2.5 x the hidden interval (DESIGN 7.7).
            fresh_for: Duration::from_mins(75),
            needs_login: false,
        }
    }

    fn fetch(&self) -> FetchFuture<'_> {
        Box::pin(std::future::ready(Err(SourceError::Unavailable(
            Unavailable::Internal,
        ))))
    }
}
