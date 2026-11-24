//! The DeepSeek balance as a source (DESIGN 6.4). The type and the descriptor are fixed
//! here; T05 writes the body.

use crate::http::{Endpoint, HttpClient};
use crate::secrets::{deepseek_key, KeySources};
use jev_usage_core::meter::Reading;
use jev_usage_core::parse::deepseek::parse_balance;
use jev_usage_core::port::{
    Cost, FetchFuture, SourceDescriptor, SourceError, SourceId, Trigger, UsageSource,
};
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::time::Clock;
use jev_usage_core::usage::{Setup, Unavailable};
use std::sync::Arc;
use std::time::Duration;

/// Reads the balance with a key that lives for one fetch.
pub struct DeepSeekBalanceSource {
    http: Arc<HttpClient>,
    clock: Arc<dyn Clock>,
    keys: Arc<dyn Fn() -> KeySources + Send + Sync>,
}

impl DeepSeekBalanceSource {
    /// A source that asks `keys` for the key at each fetch.
    pub fn new(
        http: Arc<HttpClient>,
        clock: Arc<dyn Clock>,
        keys: Arc<dyn Fn() -> KeySources + Send + Sync>,
    ) -> Self {
        Self { http, clock, keys }
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
        let http = Arc::clone(&self.http);
        let clock = Arc::clone(&self.clock);
        let keys = Arc::clone(&self.keys);
        Box::pin(async move {
            let key = tokio::task::spawn_blocking(move || deepseek_key(keys()))
                .await
                .map_err(|_| SourceError::Unavailable(Unavailable::Internal))?
                .ok_or(SourceError::NotConfigured(Setup::AddDeepSeekKey))?;
            let body = http
                .get_json(Endpoint::DeepSeekBalance, &key)
                .await
                .map_err(SourceError::Unavailable)?;
            drop(key);
            let meters = parse_balance(&body).map_err(SourceError::Unavailable)?;
            Ok(Reading {
                meters,
                plan: None,
                observed_at: clock.now(),
            })
        })
    }
}
