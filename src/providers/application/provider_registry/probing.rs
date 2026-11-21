//! Finding out who is available: the startup probe (live or from the cache), `/providers
//! refresh`, and folding the answers of the background retries into the runtime.

use std::sync::Arc;

use crate::backend::ProviderId;
use crate::providers::application::ports::ProviderHost;
use crate::providers::domain::availability;
use crate::providers::domain::availability::{Availability, State};
use crate::providers::domain::probe_source::ProbeSource;
use crate::providers::domain::reset_time::cooldowns::Cooldowns;
use crate::providers::domain::usability::allowances_for;
use crate::shared::ALL_PROVIDERS;

use super::{BlockReason, LateMerge, LateProbes, ProbeCache, Runtime};

impl Runtime {
    /// Probe the machine and load persisted cooldowns.
    pub async fn start(host: Arc<dyn ProviderHost>) -> Self {
        let avail = availability::probe_all(|p| host.probe(p)).await;
        let cooldown_path = cooldown_path(&*host);
        let cooldowns = cooldown_path.as_deref().map(Cooldowns::load).unwrap_or_default();
        let allow = allowances_for(host.env("JEV_ALLOW_CREDITS").as_deref(), &avail);
        let mut runtime = Self::build(avail, cooldowns, allow, crate::catalog::domain::model_table::Overrides::from_setting(host.env("JEV_CHAINS")), cooldown_path, host);
        runtime.seed_usage();
        runtime
    }

    /// Fill in what is known about each provider's usage before any turn: the readings an
    /// earlier run saved, Codex's own latest reading from its session files (fresher than
    /// anything saved), and DeepSeek's balance in the background. From now on every new
    /// reading is saved to `plan_path`.
    fn seed_usage(&mut self) {
        let plan_path = self.host.plan_path();
        if let Some(path) = &plan_path {
            self.plan = self.host.load_plans(path);
        }
        self.plan_path = plan_path;
        self.refresh_codex_plan();
        self.refresh_balance();
    }

    /// Like [`Runtime::start`], but a recent probe answer saved by an earlier run is used at
    /// once and the provider CLIs are asked again in the background; [`Runtime::absorb_late`]
    /// then corrects whatever changed. Without a usable cache the probe runs now, as in
    /// `start`, and its answer is saved for the next run. Says where the answer came from.
    pub async fn start_cached(cache: Option<ProbeCache>, host: Arc<dyn ProviderHost>) -> (Self, ProbeSource) {
        let now = host.now_ms();
        let hit = cache.as_ref().and_then(|c| host.load_probe_cache(&c.path, now, c.fingerprint));
        let Some(cached) = hit else {
            let runtime = Self::start(host).await;
            if let Some(c) = &cache {
                runtime.host.save_probe_cache(&c.path, runtime.host.now_ms(), c.fingerprint, &runtime.avail);
            }
            return (runtime, ProbeSource::Live);
        };
        let cooldown_path = cooldown_path(&*host);
        let cooldowns = cooldown_path.as_deref().map(Cooldowns::load).unwrap_or_default();
        let allow = allowances_for(host.env("JEV_ALLOW_CREDITS").as_deref(), &cached);
        let mut runtime = Self::build(cached.clone(), cooldowns, allow, crate::catalog::domain::model_table::Overrides::from_setting(host.env("JEV_CHAINS")), cooldown_path, host);
        runtime.seed_usage();
        runtime.refresh_in_background(cached, cache);
        (runtime, ProbeSource::Cache)
    }

    /// Ask every provider again, off the prompt's path. Each answer reaches
    /// [`Runtime::absorb_late`] as it arrives; the cache is rewritten when all are in.
    fn refresh_in_background(&mut self, cached: Vec<Availability>, cache: Option<ProbeCache>) {
        let host = self.host.clone();
        let (tx, answers) = tokio::sync::mpsc::unbounded_channel();
        self.late = Some(LateProbes { answers, merge: LateMerge::Refresh, on_ready: Box::new(|_| {}) });
        tokio::spawn(async move {
            let mut latest = cached;
            availability::retry::retry_unknown(ALL_PROVIDERS.to_vec(), availability::PROBE_CAP, {
                let host = host.clone();
                move |p| host.probe(p)
            }, |answer| {
                availability::retry::merge_refresh(&mut latest, answer.clone());
                let _ = tx.send(answer);
            })
            .await;
            if let Some(c) = cache {
                host.save_probe_cache(&c.path, host.now_ms(), c.fingerprint, &latest);
            }
        });
    }

    /// Probe again (for `/providers refresh`). Cooldowns, unusable models and plan
    /// readings are kept: a re-login does not end a usage limit. A provider that failed
    /// its login is offered again once the probe sees it logged in. The per-token
    /// allowance is recomputed because a key may have been added.
    pub async fn refresh(&mut self) {
        let host = self.host.clone();
        self.apply_probe(availability::probe_all(|p| host.probe(p)).await);
    }

    /// The pure half of [`Runtime::refresh`].
    pub(super) fn apply_probe(&mut self, avail: Vec<Availability>) {
        self.avail = avail;
        self.allow = allowances_for(self.host.env("JEV_ALLOW_CREDITS").as_deref(), &self.avail);
        let avail = &self.avail;
        self.blocked.retain(|p, reason| {
            *reason != BlockReason::Auth || !avail.iter().any(|a| a.provider == *p && a.state == State::Ready)
        });
    }

    /// Re-check once, in the background, the providers whose startup probe could not tell
    /// (a lazy wrapper's first run can outlast the startup cap), with a longer cap. Startup
    /// does not wait for it. The answers reach this runtime through [`Runtime::absorb_late`],
    /// where the caller holds `&mut self`, so a running turn never sees routing change under it.
    /// `on_ready` runs there too, once per provider that was Unknown and is now Ready: a
    /// late answer a `/providers refresh` already settled is refused, and so is not announced.
    pub fn retry_slow_probes(&mut self, on_ready: impl Fn(&Availability) + Send + Sync + 'static) {
        let host = self.host.clone();
        self.retry_slow_probes_with(availability::retry::RETRY_CAP, move |p| host.probe(p), on_ready);
    }

    pub(super) fn retry_slow_probes_with<P, Fut>(&mut self, cap: std::time::Duration, probe: P, on_ready: impl Fn(&Availability) + Send + Sync + 'static)
    where
        P: Fn(ProviderId) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Availability> + Send + 'static,
    {
        let providers = availability::retry::retry_candidates(&self.avail);
        if providers.is_empty() {
            // A background refresh may be running; the caller still wants to hear what it finds.
            if let Some(late) = self.late.as_mut() {
                late.on_ready = Box::new(on_ready);
            }
            return;
        }
        let (tx, answers) = tokio::sync::mpsc::unbounded_channel();
        self.late = Some(LateProbes { answers, merge: LateMerge::FillUnknown, on_ready: Box::new(on_ready) });
        tokio::spawn(availability::retry::retry_unknown(providers, cap, probe, move |answer| {
            let _ = tx.send(answer);
        }));
    }

    /// Apply what the background retry has found so far, announce (through the `on_ready`
    /// given to [`Runtime::retry_slow_probes`]) each provider that became Ready by it, and
    /// return whether anything changed. Call it where no turn is running (before a command
    /// or a turn starts).
    pub fn absorb_late(&mut self) -> bool {
        use tokio::sync::mpsc::error::TryRecvError;
        let Some(late) = self.late.as_mut() else { return false };
        let merge = late.merge;
        let mut merged = self.avail.clone();
        let mut applied = false;
        let mut now_ready = Vec::new();
        let finished = loop {
            match late.answers.try_recv() {
                Ok(answer) => {
                    let was_ready = merged.iter().any(|a| a.provider == answer.provider && a.state == State::Ready);
                    let changed = match merge {
                        LateMerge::FillUnknown => availability::retry::merge_late(&mut merged, answer.clone()),
                        LateMerge::Refresh => availability::retry::merge_refresh(&mut merged, answer.clone()),
                    };
                    applied |= changed;
                    if changed && answer.state == State::Ready && !was_ready {
                        now_ready.push(answer);
                    }
                }
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };
        if applied {
            self.apply_probe(merged);
        }
        if let Some(late) = &self.late {
            now_ready.iter().for_each(|a| (late.on_ready)(a));
        }
        if finished {
            self.late = None;
        }
        applied
    }
}

/// `<state_dir>/cooldowns.json`, or `None` if there is no state directory.
pub fn cooldown_path(host: &dyn ProviderHost) -> Option<std::path::PathBuf> {
    Some(host.state_dir()?.join("cooldowns.json"))
}
