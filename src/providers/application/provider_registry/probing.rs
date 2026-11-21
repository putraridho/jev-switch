//! Finding out who is available: the startup probe (live or from the cache), `/providers
//! refresh`, and folding the answers of the background retries into the runtime.

use std::path::PathBuf;

use crate::backend::ProviderId;
use crate::catalog::domain::model_table::Overrides;
use crate::providers::domain::availability;
use crate::providers::domain::availability::{Availability, State};
use crate::providers::domain::probe_source::ProbeSource;
use crate::providers::domain::reset_time as cooldown;
use crate::providers::domain::reset_time::Cooldowns;
use crate::providers::domain::usability::allowances_for;
use crate::providers::infrastructure::probe_cache as probecache;
use crate::shared::ALL_PROVIDERS;

use super::{BlockReason, LateMerge, LateProbes, ProbeCache, Runtime};

impl Runtime {
    /// Probe the machine and load persisted cooldowns.
    pub async fn start() -> Self {
        let avail = availability::probe_all().await;
        let cooldown_path = cooldown::default_path();
        let cooldowns = cooldown_path.as_deref().map(Cooldowns::load).unwrap_or_default();
        let allow = allowances_for(crate::shared::infrastructure::env_file::lookup("JEV_ALLOW_CREDITS").as_deref(), &avail);
        let mut runtime = Self::build(avail, cooldowns, allow, Overrides::from_env(), cooldown_path);
        runtime.seed_usage(crate::providers::infrastructure::plan_store::default_path());
        runtime
    }

    /// Fill in what is known about each provider's usage before any turn: the readings an
    /// earlier run saved, Codex's own latest reading from its session files (fresher than
    /// anything saved), and DeepSeek's balance in the background. From now on every new
    /// reading is saved to `plan_path`.
    fn seed_usage(&mut self, plan_path: Option<PathBuf>) {
        if let Some(path) = &plan_path {
            self.plan = crate::providers::infrastructure::plan_store::load(path);
        }
        self.plan_path = plan_path;
        self.refresh_codex_plan();
        self.refresh_balance();
    }

    /// Like [`Runtime::start`], but a recent probe answer saved by an earlier run is used at
    /// once and the provider CLIs are asked again in the background; [`Runtime::absorb_late`]
    /// then corrects whatever changed. Without a usable cache the probe runs now, as in
    /// `start`, and its answer is saved for the next run. Says where the answer came from.
    pub async fn start_cached(cache: Option<ProbeCache>) -> (Self, ProbeSource) {
        let now = crate::shared::infrastructure::jsonl_log::now_ms();
        let hit = cache.as_ref().and_then(|c| probecache::load(&c.path, now, c.fingerprint));
        let Some(cached) = hit else {
            let runtime = Self::start().await;
            if let Some(c) = &cache {
                probecache::save(&c.path, crate::shared::infrastructure::jsonl_log::now_ms(), c.fingerprint, &runtime.avail);
            }
            return (runtime, ProbeSource::Live);
        };
        let cooldown_path = cooldown::default_path();
        let cooldowns = cooldown_path.as_deref().map(Cooldowns::load).unwrap_or_default();
        let allow = allowances_for(crate::shared::infrastructure::env_file::lookup("JEV_ALLOW_CREDITS").as_deref(), &cached);
        let mut runtime = Self::build(cached.clone(), cooldowns, allow, Overrides::from_env(), cooldown_path);
        runtime.seed_usage(crate::providers::infrastructure::plan_store::default_path());
        runtime.refresh_in_background(cached, cache);
        (runtime, ProbeSource::Cache)
    }

    /// Ask every provider again, off the prompt's path. Each answer reaches
    /// [`Runtime::absorb_late`] as it arrives; the cache is rewritten when all are in.
    fn refresh_in_background(&mut self, cached: Vec<Availability>, cache: Option<ProbeCache>) {
        let (tx, answers) = tokio::sync::mpsc::unbounded_channel();
        self.late = Some(LateProbes { answers, merge: LateMerge::Refresh, on_ready: Box::new(|_| {}) });
        tokio::spawn(async move {
            let mut latest = cached;
            availability::retry_unknown(ALL_PROVIDERS.to_vec(), availability::PROBE_CAP, availability::probe_provider, |answer| {
                availability::merge_refresh(&mut latest, answer.clone());
                let _ = tx.send(answer);
            })
            .await;
            if let Some(c) = cache {
                probecache::save(&c.path, crate::shared::infrastructure::jsonl_log::now_ms(), c.fingerprint, &latest);
            }
        });
    }

    /// Probe again (for `/providers refresh`). Cooldowns, unusable models and plan
    /// readings are kept: a re-login does not end a usage limit. A provider that failed
    /// its login is offered again once the probe sees it logged in. The per-token
    /// allowance is recomputed because a key may have been added.
    pub async fn refresh(&mut self) {
        self.apply_probe(availability::probe_all().await);
    }

    /// The pure half of [`Runtime::refresh`].
    pub(super) fn apply_probe(&mut self, avail: Vec<Availability>) {
        self.avail = avail;
        self.allow = allowances_for(crate::shared::infrastructure::env_file::lookup("JEV_ALLOW_CREDITS").as_deref(), &self.avail);
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
        self.retry_slow_probes_with(availability::RETRY_CAP, availability::probe_provider, on_ready);
    }

    pub(super) fn retry_slow_probes_with<P, Fut>(&mut self, cap: std::time::Duration, probe: P, on_ready: impl Fn(&Availability) + Send + Sync + 'static)
    where
        P: Fn(ProviderId) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Availability> + Send + 'static,
    {
        let providers = availability::retry_candidates(&self.avail);
        if providers.is_empty() {
            // A background refresh may be running; the caller still wants to hear what it finds.
            if let Some(late) = self.late.as_mut() {
                late.on_ready = Box::new(on_ready);
            }
            return;
        }
        let (tx, answers) = tokio::sync::mpsc::unbounded_channel();
        self.late = Some(LateProbes { answers, merge: LateMerge::FillUnknown, on_ready: Box::new(on_ready) });
        tokio::spawn(availability::retry_unknown(providers, cap, probe, move |answer| {
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
                        LateMerge::FillUnknown => availability::merge_late(&mut merged, answer.clone()),
                        LateMerge::Refresh => availability::merge_refresh(&mut merged, answer.clone()),
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
