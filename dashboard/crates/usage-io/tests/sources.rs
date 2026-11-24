//! The source descriptors the design fixes (DESIGN 6, 7.7) and the placeholder source (6.5).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use jev_usage_core::port::{Cost, SourceError, SourceId, Trigger, UsageSource};
use jev_usage_core::provider::ProviderKey;
use jev_usage_io::clock::FakeClock;
use jev_usage_io::http::HttpClient;
use jev_usage_io::probe::{FactsCell, LoginFacts};
use jev_usage_io::proc::ProcRunner;
use jev_usage_io::secrets::KeySources;
use jev_usage_io::sources::claude_live::ClaudeLiveSource;
use jev_usage_io::sources::claude_log::ClaudeGatewayLogSource;
use jev_usage_io::sources::codex::CodexAppServerSource;
use jev_usage_io::sources::deepseek::DeepSeekBalanceSource;
use jev_usage_io::sources::placeholder::PlaceholderSource;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn now<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(value) => value,
        std::task::Poll::Pending => panic!("the future was not ready"),
    }
}

const MIN: u64 = 60;

#[test]
fn placeholders_exist_for_exactly_the_three_providers_without_a_source() {
    let cases = [
        (ProviderKey::Gemini, SourceId::GeminiNone),
        (ProviderKey::Antigravity, SourceId::AntigravityNone),
        (ProviderKey::Grok, SourceId::GrokNone),
    ];
    for (provider, id) in cases {
        let source = PlaceholderSource::new(provider).unwrap();
        let d = source.descriptor();
        assert_eq!((d.id, d.provider), (id, provider));
        assert_eq!(d.trigger, Trigger::OnSignal, "never on a timer");
        assert_eq!(d.cost, Cost::Free);
        assert!(!d.needs_login);
        assert_eq!(now(source.fetch()), Err(SourceError::Unsupported));
        assert_eq!(
            now(source.fetch()),
            Err(SourceError::Unsupported),
            "every time"
        );
        let shared: Arc<dyn UsageSource> = Arc::new(source);
        assert_eq!(now(shared.fetch()), Err(SourceError::Unsupported));
    }
    for provider in [
        ProviderKey::Claude,
        ProviderKey::Codex,
        ProviderKey::DeepSeek,
    ] {
        assert_eq!(
            PlaceholderSource::new(provider),
            None,
            "{provider:?} has a real source"
        );
    }
}

#[test]
fn real_sources_carry_the_design_constants() {
    let clock = FakeClock::at(0);
    let facts = FactsCell::new(LoginFacts::unknown());
    let runner = Arc::new(ProcRunner::system(PathBuf::from("/nonexistent")));

    let log = ClaudeGatewayLogSource::new(None, clock.clone(), facts.clone()).descriptor();
    assert_eq!(
        (log.id, log.provider),
        (SourceId::ClaudeGatewayLog, ProviderKey::Claude)
    );
    assert_eq!(log.trigger, Trigger::OnSignal);
    assert_eq!(log.cost, Cost::Free);
    assert_eq!(log.timeout, Duration::from_secs(5));
    assert_eq!(log.fresh_for, Duration::from_secs(10 * MIN));
    assert!(!log.needs_login);

    let live = ClaudeLiveSource::new(runner.clone(), clock.clone(), facts.clone()).descriptor();
    assert_eq!(
        (live.id, live.provider),
        (SourceId::ClaudeLive, ProviderKey::Claude)
    );
    assert_eq!(
        live.trigger,
        Trigger::Manual,
        "the spending source never runs from a timer"
    );
    assert_eq!(live.cost, Cost::SpendsQuota);
    assert_eq!(live.timeout.as_secs(), 60);
    assert_eq!(live.fresh_for, Duration::from_secs(10 * MIN));
    assert!(live.needs_login);

    let codex = CodexAppServerSource::new(runner, clock.clone(), facts).descriptor();
    assert_eq!(
        (codex.id, codex.provider),
        (SourceId::CodexAppServer, ProviderKey::Codex)
    );
    let (visible, hidden) = (Duration::from_secs(5 * MIN), Duration::from_secs(15 * MIN));
    assert_eq!(codex.trigger, Trigger::Interval { visible, hidden });
    assert_eq!(codex.cost, Cost::Free);
    assert_eq!(codex.timeout, Duration::from_secs(15));
    assert_eq!(
        codex.fresh_for,
        hidden.mul_f64(2.5),
        "2.5 x the hidden interval: 37.5 min"
    );
    assert!(codex.needs_login);

    let http = Arc::new(HttpClient::new().unwrap());
    let keys: Arc<dyn Fn() -> KeySources + Send + Sync> = Arc::new(|| KeySources {
        env: None,
        env_file: None,
    });
    let deepseek = DeepSeekBalanceSource::new(http, clock, keys).descriptor();
    assert_eq!(
        (deepseek.id, deepseek.provider),
        (SourceId::DeepSeekBalance, ProviderKey::DeepSeek)
    );
    let (visible, hidden) = (Duration::from_secs(10 * MIN), Duration::from_secs(30 * MIN));
    assert_eq!(deepseek.trigger, Trigger::Interval { visible, hidden });
    assert_eq!(deepseek.cost, Cost::Free);
    assert_eq!(deepseek.timeout, Duration::from_secs(10));
    assert_eq!(
        deepseek.fresh_for,
        hidden.mul_f64(2.5),
        "2.5 x the hidden interval: 75 min"
    );
    assert!(!deepseek.needs_login);
}
