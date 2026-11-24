//! The narrow login probe (DESIGN 8.6): what the app knows about the CLIs' logins. The types
//! are complete here; T04 writes `CliProbe`'s body.

use crate::proc::ProcRunner;
use jev_usage_core::label::Label;
use jev_usage_core::provider::ProviderKey;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

/// Whether a provider's CLI can be used.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Readiness {
    /// Installed and signed in.
    Ready,
    /// No executable on disk.
    NotInstalled,
    /// Installed, not signed in.
    NotLoggedIn,
    /// Not probed, or the probe timed out or could not read the answer.
    Unknown,
}

/// A readiness and a plan name per provider, and nothing else: the account's e-mail cannot
/// be held here, so it cannot be displayed, stored, logged or sent (DESIGN 8.3).
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct LoginFacts {
    /// One entry per provider, in `ProviderKey::ALL` order.
    entries: [(Readiness, Option<Label>); 6],
}

impl LoginFacts {
    /// Before the first probe: every provider `Unknown`, no plan.
    pub fn unknown() -> Self {
        Self {
            entries: std::array::from_fn(|_| (Readiness::Unknown, None)),
        }
    }

    /// These facts with one provider's readiness and plan replaced.
    pub fn with(mut self, p: ProviderKey, r: Readiness, plan: Option<Label>) -> Self {
        if let Some(entry) = self.entries.get_mut(Self::slot(p)) {
            *entry = (r, plan);
        }
        self
    }

    /// The provider's readiness.
    pub fn readiness(&self, p: ProviderKey) -> Readiness {
        self.entries
            .get(Self::slot(p))
            .map_or(Readiness::Unknown, |entry| entry.0)
    }

    /// The provider's plan name, when the probe learned one.
    pub fn plan(&self, p: ProviderKey) -> Option<Label> {
        self.entries
            .get(Self::slot(p))
            .and_then(|entry| entry.1.clone())
    }

    /// The provider's position in `ProviderKey::ALL`.
    fn slot(p: ProviderKey) -> usize {
        match p {
            ProviderKey::Claude => 0,
            ProviderKey::Codex => 1,
            ProviderKey::DeepSeek => 2,
            ProviderKey::Gemini => 3,
            ProviderKey::Antigravity => 4,
            ProviderKey::Grok => 5,
        }
    }
}

/// The future a probe returns.
pub type ProbeFuture<'a> = Pin<Box<dyn Future<Output = LoginFacts> + Send + 'a>>;

/// The one seam to the CLIs' login state, so the service is testable. `providers` are the
/// ones to probe (not hidden, with a `needs_login` source); the others stay `Unknown`.
pub trait LoginProbe: Send + Sync {
    /// Asks the CLIs of `providers` whether they are installed and signed in.
    fn probe(&self, providers: Vec<ProviderKey>) -> ProbeFuture<'_>;
}

/// The production probe: `codex login status` and `claude auth status` through `proc`,
/// under every rule of DESIGN 8.6. It opens no file of any CLI.
#[derive(Debug)]
pub struct CliProbe {
    _runner: Arc<ProcRunner>,
}

impl CliProbe {
    /// A probe that starts its two status commands through `runner`.
    pub fn new(runner: Arc<ProcRunner>) -> Self {
        Self { _runner: runner }
    }
}

impl LoginProbe for CliProbe {
    fn probe(&self, providers: Vec<ProviderKey>) -> ProbeFuture<'_> {
        let _ = providers;
        Box::pin(std::future::ready(LoginFacts::unknown()))
    }
}

/// A shared read handle on the latest login facts. Written only by the service.
#[derive(Debug)]
pub struct FactsCell(RwLock<LoginFacts>);

impl FactsCell {
    /// A cell that holds `f`.
    pub fn new(f: LoginFacts) -> Arc<Self> {
        Arc::new(Self(RwLock::new(f)))
    }

    /// The current facts.
    pub fn get(&self) -> LoginFacts {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the facts.
    pub fn set(&self, f: LoginFacts) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = f;
    }
}

#[cfg(feature = "test-support")]
pub use scripted::ScriptedProbe;

#[cfg(feature = "test-support")]
mod scripted {
    use super::{LoginFacts, LoginProbe, ProbeFuture};
    use jev_usage_core::provider::ProviderKey;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// Answers in order, repeating the last one, and counts calls (feature `test-support`).
    /// With no answers it says `LoginFacts::unknown()`.
    #[derive(Debug)]
    pub struct ScriptedProbe {
        answers: Vec<LoginFacts>,
        calls: AtomicU32,
    }

    impl ScriptedProbe {
        /// A probe that gives `answers` one per call.
        pub fn new(answers: Vec<LoginFacts>) -> Arc<Self> {
            Arc::new(Self {
                answers,
                calls: AtomicU32::new(0),
            })
        }

        /// How often `probe` was called.
        pub fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl LoginProbe for ScriptedProbe {
        fn probe(&self, providers: Vec<ProviderKey>) -> ProbeFuture<'_> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let index = usize::try_from(call).unwrap_or(usize::MAX);
            let answer = self
                .answers
                .get(index)
                .or_else(|| self.answers.last())
                .cloned()
                .unwrap_or_else(LoginFacts::unknown);
            let selected = providers
                .into_iter()
                .fold(LoginFacts::unknown(), |facts, provider| {
                    facts.with(provider, answer.readiness(provider), answer.plan(provider))
                });
            Box::pin(std::future::ready(selected))
        }
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

    fn label(s: &str) -> Label {
        Label::parse(s).unwrap()
    }

    #[test]
    fn unknown_is_unknown_everywhere() {
        let facts = LoginFacts::unknown();
        for p in ProviderKey::ALL {
            assert_eq!(facts.readiness(p), Readiness::Unknown, "{p:?}");
            assert_eq!(facts.plan(p), None, "{p:?}");
        }
    }

    #[test]
    fn with_sets_exactly_one_provider() {
        for target in ProviderKey::ALL {
            let facts = LoginFacts::unknown().with(target, Readiness::Ready, Some(label("max_5x")));
            for p in ProviderKey::ALL {
                if p == target {
                    assert_eq!(facts.readiness(p), Readiness::Ready);
                    assert_eq!(facts.plan(p), Some(label("max_5x")));
                } else {
                    assert_eq!(
                        facts.readiness(p),
                        Readiness::Unknown,
                        "{target:?} changed {p:?}"
                    );
                    assert_eq!(facts.plan(p), None);
                }
            }
        }
    }

    #[test]
    fn with_replaces_and_chains() {
        let facts = LoginFacts::unknown()
            .with(ProviderKey::Claude, Readiness::Ready, Some(label("max_5x")))
            .with(ProviderKey::Codex, Readiness::NotLoggedIn, None)
            .with(ProviderKey::Claude, Readiness::NotInstalled, None);
        assert_eq!(
            facts.readiness(ProviderKey::Claude),
            Readiness::NotInstalled
        );
        assert_eq!(facts.plan(ProviderKey::Claude), None);
        assert_eq!(facts.readiness(ProviderKey::Codex), Readiness::NotLoggedIn);
    }

    #[test]
    fn slots_follow_provider_order() {
        for (index, p) in ProviderKey::ALL.into_iter().enumerate() {
            assert_eq!(LoginFacts::slot(p), index);
        }
    }

    #[test]
    fn debug_holds_only_readiness_names_and_labels() {
        let facts = LoginFacts::unknown()
            .with(ProviderKey::Claude, Readiness::Ready, Some(label("max_5x")))
            .with(ProviderKey::Codex, Readiness::NotLoggedIn, None)
            .with(ProviderKey::Grok, Readiness::NotInstalled, None);
        assert_eq!(
            format!("{facts:?}"),
            "LoginFacts { entries: [(Ready, Some(Label(\"max_5x\"))), (NotLoggedIn, None), (Unknown, None), \
             (Unknown, None), (Unknown, None), (NotInstalled, None)] }"
        );
    }

    #[test]
    fn facts_cell_gets_what_was_set() {
        let cell = FactsCell::new(LoginFacts::unknown());
        assert_eq!(cell.get(), LoginFacts::unknown());
        let ready = LoginFacts::unknown().with(ProviderKey::Codex, Readiness::Ready, None);
        cell.set(ready.clone());
        assert_eq!(cell.get(), ready);
        let other = Arc::clone(&cell);
        std::thread::spawn(move || other.set(LoginFacts::unknown()))
            .join()
            .unwrap();
        assert_eq!(cell.get(), LoginFacts::unknown());
    }

    fn now<T>(future: impl Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("the future was not ready"),
        }
    }

    #[test]
    fn scripted_probe_answers_in_order_repeats_the_last_and_counts() {
        let first = LoginFacts::unknown();
        let second = LoginFacts::unknown().with(ProviderKey::Codex, Readiness::Ready, None);
        let probe = ScriptedProbe::new(vec![first.clone(), second.clone()]);
        assert_eq!(probe.calls(), 0);
        assert_eq!(now(probe.probe(vec![ProviderKey::Codex])), first);
        assert_eq!(now(probe.probe(vec![ProviderKey::Codex])), second);
        assert_eq!(now(probe.probe(vec![])), LoginFacts::unknown());
        assert_eq!(
            now(probe.probe(vec![ProviderKey::Claude])),
            LoginFacts::unknown()
        );
        assert_eq!(probe.calls(), 4);
        let empty = ScriptedProbe::new(vec![]);
        assert_eq!(
            now(empty.probe(vec![ProviderKey::Claude])),
            LoginFacts::unknown()
        );
        assert_eq!(empty.calls(), 1);
        let shared: Arc<dyn LoginProbe> = probe;
        assert_eq!(now(shared.probe(vec![])), LoginFacts::unknown());
    }

    #[test]
    fn cli_probe_stub_answers_unknown() {
        let runner = Arc::new(ProcRunner::system(std::path::PathBuf::from("/nonexistent")));
        let probe = CliProbe::new(runner);
        assert_eq!(
            now(probe.probe(vec![ProviderKey::Claude, ProviderKey::Codex])),
            LoginFacts::unknown()
        );
    }
}
