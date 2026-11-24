//! The usage service: one task that owns all mutable state (DESIGN 7.1, 7.2). The types
//! are frozen; T08 writes the bodies.

use crate::probe::{FactsCell, LoginProbe};
use crate::timer::WallTimer;
use jev_usage_core::alerts::AlertMemory;
use jev_usage_core::assess::{self, FreshnessPolicy, SourceState};
use jev_usage_core::ipc::{
    CommandError, Diagnostics, HistoryPoint, HistoryQuery, HistoryRange, HistorySeries,
    RefreshTarget, RefreshView, SeriesUnit, Settings, SettingsPatch, Snapshot, SourceDiag,
};
use jev_usage_core::meter::Reading;
use jev_usage_core::port::{
    Cost, HistoryStore, Notifier, SettingsStore, SourceError, SourceId, StoreError, Trigger,
    UsageSource,
};
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::time::{Clock, UnixMillis, UnixSeconds};
use jev_usage_core::usage::{Unavailable, Usage};
use jev_usage_core::view;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

/// The source of backoff jitter, injected so schedules are testable.
pub trait Jitter: Send + Sync {
    /// The next value in `0.0..1.0`.
    fn next(&self) -> f64;
}

/// The first-run history import of one provider; `run` executes on a blocking thread.
pub struct Backfill {
    /// The provider whose live recording waits for the import.
    pub provider: ProviderKey,
    /// The import; returns samples stored.
    pub run: Box<dyn FnOnce() -> Result<u64, StoreError> + Send>,
}

/// What only the shell knows, passed to `diagnostics`.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct ShellFacts {
    /// The app's version: a compile-time constant.
    pub app_version: &'static str,
    /// Whether a tray host is present now.
    pub tray_available: bool,
}

/// Everything the service touches. All of it is injected, so every behaviour is testable
/// with fakes.
pub struct ServiceDeps {
    /// The wall clock.
    pub clock: Arc<dyn Clock>,
    /// The wall-clock timer: the loop's one wait.
    pub timer: Arc<dyn WallTimer>,
    /// Backoff jitter.
    pub jitter: Arc<dyn Jitter>,
    /// Every source, from `registry::build`.
    pub sources: Vec<Arc<dyn UsageSource>>,
    /// The CLI login probe.
    pub probe: Arc<dyn LoginProbe>,
    /// The login facts the sources read; the service merges probe answers into it.
    pub facts: Arc<FactsCell>,
    /// The history store.
    pub store: Arc<dyn HistoryStore>,
    /// The settings store.
    pub settings: Arc<dyn SettingsStore>,
    /// Where alerts go.
    pub notifier: Arc<dyn Notifier>,
    /// Signals from push sources (the file watcher).
    pub signals: mpsc::Receiver<SourceId>,
    /// The first-run backfill; `None` when already done.
    pub backfill: Option<Backfill>,
}

/// A command the shell sends the loop through the handle.
enum Command {
    Refresh(RefreshTarget, oneshot::Sender<Result<(), CommandError>>),
    History(
        HistoryQuery,
        oneshot::Sender<Result<HistorySeries, CommandError>>,
    ),
    UpdateSettings(
        SettingsPatch,
        oneshot::Sender<Result<Settings, CommandError>>,
    ),
    SetVisible(bool),
    Diagnostics(ShellFacts, oneshot::Sender<Diagnostics>),
    Shutdown,
}

/// The handle the shell holds. Cloneable; dropping every clone stops the task.
#[derive(Clone, Debug)]
pub struct ServiceHandle {
    snapshot: watch::Sender<Snapshot>,
    settings: watch::Sender<Settings>,
    cmd: mpsc::Sender<Command>,
    /// Wakes the fetch wait so a shutdown does not sit behind a hanging source.
    stop: Arc<tokio::sync::Notify>,
}

/// Aborts its task when dropped, so cancelling a fetch drops the fetch's process-group guard.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl ServiceHandle {
    /// The latest snapshot: a lock-free read of the watch channel, never IO.
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.borrow().clone()
    }

    /// A receiver that sees every new snapshot.
    pub fn subscribe(&self) -> watch::Receiver<Snapshot> {
        self.snapshot.subscribe()
    }

    /// Queues the fetches and returns; never waits for a fetch to finish (DESIGN 7.2).
    pub async fn refresh(&self, target: RefreshTarget) -> Result<(), CommandError> {
        let (tx, rx) = oneshot::channel();
        if self.cmd.send(Command::Refresh(target, tx)).await.is_err() {
            return Err(CommandError::Internal);
        }
        rx.await.unwrap_or(Err(CommandError::Internal))
    }

    /// One meter's history over a range: at most 600 points.
    pub async fn history(&self, q: HistoryQuery) -> Result<HistorySeries, CommandError> {
        let (tx, rx) = oneshot::channel();
        if self.cmd.send(Command::History(q, tx)).await.is_err() {
            return Err(CommandError::Internal);
        }
        rx.await.unwrap_or(Err(CommandError::Internal))
    }

    /// The current settings.
    pub fn settings(&self) -> Settings {
        self.settings.borrow().clone()
    }

    /// A receiver that sees every settings change.
    pub fn subscribe_settings(&self) -> watch::Receiver<Settings> {
        self.settings.subscribe()
    }

    /// Validates and applies a patch, saves it, and returns the new settings.
    pub async fn update_settings(&self, patch: SettingsPatch) -> Result<Settings, CommandError> {
        let (tx, rx) = oneshot::channel();
        if self
            .cmd
            .send(Command::UpdateSettings(patch, tx))
            .await
            .is_err()
        {
            return Err(CommandError::Internal);
        }
        rx.await.unwrap_or(Err(CommandError::Internal))
    }

    /// Tells the service whether the window is visible (it polls less while hidden).
    pub fn set_visible(&self, visible: bool) {
        let _ = self.cmd.try_send(Command::SetVisible(visible));
    }

    /// Version, per-source state and store size; reads store statistics off the loop.
    pub async fn diagnostics(&self, shell: ShellFacts) -> Diagnostics {
        let (tx, rx) = oneshot::channel();
        if self
            .cmd
            .send(Command::Diagnostics(shell, tx))
            .await
            .is_err()
        {
            return empty_diagnostics(shell);
        }
        rx.await.unwrap_or_else(|_| empty_diagnostics(shell))
    }

    /// Cancels in-flight fetches (killing their process groups) and stops the task.
    /// Bounded: 2 s.
    pub async fn shutdown(&self) {
        self.stop.notify_one();
        let _ = self.cmd.send(Command::Shutdown).await;
        // The loop drops its command receiver as it ends, after the fetches are cancelled.
        let _ = tokio::time::timeout(Duration::from_secs(2), self.cmd.closed()).await;
    }
}

fn empty_diagnostics(shell: ShellFacts) -> Diagnostics {
    Diagnostics {
        app_version: shell.app_version,
        sources: Vec::new(),
        history_bytes: 0,
        history_samples: 0,
        tray_available: shell.tray_available,
    }
}

/// One source's scheduling state, owned by the loop.
struct SourceMeta {
    id: SourceId,
    provider: ProviderKey,
    trigger: Trigger,
    cost: Cost,
    timeout: Duration,
    fresh_for: Duration,
    needs_login: bool,
    failures: u32,
    next_due: UnixMillis,
    in_flight: bool,
    last_ok_at: Option<UnixMillis>,
    last_error: Option<Unavailable>,
    signal_pending: bool,
}

/// The provider's merged state.
struct ProviderState {
    state: SourceState,
    fresh_for: Duration,
    refresh: RefreshView,
}

fn slot_of(provider: ProviderKey) -> usize {
    ProviderKey::ALL
        .iter()
        .position(|p| *p == provider)
        .unwrap_or(0)
}

fn provider_at(slot: usize) -> ProviderKey {
    ProviderKey::ALL
        .iter()
        .copied()
        .nth(slot)
        .unwrap_or(ProviderKey::Claude)
}

fn delay_millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Starts the service task. `rt` is the runtime the task runs on: the shell passes Tauri's,
/// because Tauri's `setup` hook is not inside a runtime and a bare `tokio::spawn` would panic.
pub fn spawn(deps: ServiceDeps, rt: &tokio::runtime::Handle) -> ServiceHandle {
    let (snapshot, _) = watch::channel(Snapshot {
        generated_at: 0,
        providers: Vec::new(),
        headline: None,
    });
    let (settings, _) = watch::channel(Settings::defaults());
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let handle = ServiceHandle {
        snapshot,
        settings,
        cmd: cmd_tx,
        stop: Arc::new(tokio::sync::Notify::new()),
    };
    rt.spawn(run(deps, cmd_rx, handle.clone()));
    handle
}

async fn run(deps: ServiceDeps, mut cmd_rx: mpsc::Receiver<Command>, handle: ServiceHandle) {
    let ServiceDeps {
        clock,
        timer,
        jitter,
        sources,
        probe,
        facts,
        store,
        settings: settings_store,
        notifier,
        mut signals,
        backfill,
    } = deps;

    let mut settings = settings_store.load();
    let mut alert_memory = store
        .load_alerts()
        .unwrap_or_else(|_| AlertMemory::default());
    let mut visible = true;
    let mut last_probe_at = clock.now();

    let mut metas: Vec<SourceMeta> = sources
        .iter()
        .map(|s| {
            let d = s.descriptor();
            SourceMeta {
                id: d.id,
                provider: d.provider,
                trigger: d.trigger,
                cost: d.cost,
                timeout: d.timeout,
                fresh_for: d.fresh_for,
                needs_login: d.needs_login,
                failures: 0,
                next_due: clock.now(),
                in_flight: false,
                last_ok_at: None,
                last_error: None,
                signal_pending: false,
            }
        })
        .collect();

    let mut providers: Vec<ProviderState> = (0..ProviderKey::ALL.len())
        .map(|_| ProviderState {
            state: SourceState {
                last_good: None,
                last_failure: None,
                attempted: false,
            },
            fresh_for: Duration::ZERO,
            refresh: RefreshView {
                in_flight: false,
                last_attempt_at: None,
                next_at: None,
                free_refresh: false,
                live_check: false,
                live_check_error: None,
            },
        })
        .collect();

    for meta in &metas {
        let slot = slot_of(meta.provider);
        if let Some(provider) = providers.get_mut(slot) {
            if meta.fresh_for > provider.fresh_for {
                provider.fresh_for = meta.fresh_for;
            }
            provider.refresh.free_refresh |= meta.cost == Cost::Free;
        }
    }
    recompute_live_check(&metas, &mut providers, &settings);

    // First-run backfill before any live fetch.
    if let Some(backfill) = backfill {
        let store = Arc::clone(&store);
        let _ = tokio::task::spawn_blocking(move || (backfill.run)()).await;
        let prune_now = UnixSeconds(clock.now().0 / 1000);
        let _ = tokio::task::spawn_blocking(move || store.prune(prune_now)).await;
    }

    // Initial probe before any gated source is fetched.
    run_probe(&probe, &facts, &settings, &metas).await;

    loop {
        recompute_live_check(&metas, &mut providers, &settings);
        let now = clock.now();
        let deadline = next_deadline(&metas, &providers, now);

        tokio::select! {
            () = wait_until(&timer, deadline) => (),
            signal = signals.recv() => {
                if let Some(id) = signal {
                    for meta in &mut metas {
                        if meta.id == id {
                            meta.signal_pending = true;
                        }
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(Command::Refresh(target, reply)) => {
                        let result = handle_refresh(&clock, &settings, target, &mut metas).await;
                        let _ = reply.send(result);
                    }
                    Some(Command::History(q, reply)) => {
                        let result = handle_history(&store, &q, now).await;
                        let _ = reply.send(result);
                    }
                    Some(Command::UpdateSettings(patch, reply)) => {
                        let result = match settings.apply(patch) {
                            Ok(next) => {
                                settings = next;
                                let to_save = settings.clone();
                                let store = Arc::clone(&settings_store);
                                let saved = tokio::task::spawn_blocking(move || store.save(&to_save)).await;
                                if saved.is_ok() {
                                    let _ = handle.settings.send(settings.clone());
                                    recompute_live_check(&metas, &mut providers, &settings);
                                    emit_snapshot(&handle, &providers, &settings, clock.now());
                                    Ok(settings.clone())
                                } else {
                                    Err(CommandError::Io)
                                }
                            }
                            Err(e) => Err(e),
                        };
                        let _ = reply.send(result);
                    }
                    Some(Command::SetVisible(v)) => {
                        if v && !visible {
                            for meta in &mut metas {
                                if matches!(meta.trigger, Trigger::OnSignal) {
                                    meta.next_due = clock.now();
                                }
                            }
                        }
                        visible = v;
                    }
                    Some(Command::Diagnostics(shell, reply)) => {
                        let diags = handle_diagnostics(&store, &shell, &metas).await;
                        let _ = reply.send(diags);
                    }
                    Some(Command::Shutdown) | None => break,
                }
            }
        }

        // Fetch every due source concurrently.
        let mut set = tokio::task::JoinSet::new();
        for (i, meta) in metas.iter_mut().enumerate() {
            if meta.in_flight || (meta.next_due.0 > clock.now().0 && !meta.signal_pending) {
                continue;
            }
            let Some(source) = sources.get(i) else {
                continue;
            };
            meta.in_flight = true;
            meta.signal_pending = false;
            let provider_key = meta.provider;
            if let Some(provider) = providers.get_mut(slot_of(provider_key)) {
                provider.refresh.in_flight = true;
            }
            let timeout = meta.timeout;
            let source = Arc::clone(source);
            let source_id = meta.id;
            set.spawn(async move {
                let began = std::time::Instant::now();
                let mut task = AbortOnDrop(tokio::spawn(async move { source.fetch().await }));
                let result = match tokio::time::timeout(timeout, &mut task.0).await {
                    Ok(Ok(r)) => r,
                    Ok(Err(_)) => Err(SourceError::Unavailable(Unavailable::Internal)),
                    Err(_) => Err(SourceError::Unavailable(Unavailable::Timeout)),
                };
                match &result {
                    Ok(_) => crate::diag::emit(crate::diag::DiagEvent::SourceOk {
                        source: source_id,
                        took_ms: u32::try_from(began.elapsed().as_millis()).unwrap_or(u32::MAX),
                    }),
                    Err(SourceError::Unavailable(reason)) => {
                        crate::diag::emit(crate::diag::DiagEvent::SourceFailed {
                            source: source_id,
                            reason: *reason,
                        });
                    }
                    Err(_) => {}
                }
                (i, result)
            });
        }

        let mut results: Vec<(usize, Result<Reading, SourceError>)> = Vec::new();
        let mut stopping = false;
        while !set.is_empty() {
            tokio::select! {
                outcome = set.join_next() => {
                    if let Some(Ok(pair)) = outcome {
                        results.push(pair);
                    }
                }
                () = handle.stop.notified() => {
                    // Cancel every fetch; dropping each one kills its process group.
                    set.abort_all();
                    while set.join_next().await.is_some() {}
                    stopping = true;
                }
            }
        }
        if stopping {
            break;
        }

        let mut alerts = Vec::new();
        for (i, result) in results {
            let Some(meta) = metas.get_mut(i) else {
                continue;
            };
            meta.in_flight = false;
            let slot = slot_of(meta.provider);
            let Some(provider) = providers.get_mut(slot) else {
                continue;
            };
            provider.refresh.in_flight = false;
            let now = clock.now();
            provider.refresh.last_attempt_at = Some(now.0);

            match result {
                Ok(mut reading) => {
                    meta.failures = 0;
                    meta.last_ok_at = Some(now);
                    meta.last_error = None;
                    meta.next_due = schedule_next(meta.trigger, now, visible);
                    if reading.observed_at.0 > now.0 {
                        reading.observed_at = now;
                    }
                    provider.state.attempted = true;
                    merge_reading(&mut provider.state, &reading);
                    let store = Arc::clone(&store);
                    let record_provider = meta.provider;
                    let r = reading.clone();
                    let _ = tokio::task::spawn_blocking(move || store.record(record_provider, &r))
                        .await;
                    let (next_memory, mut step_alerts) = jev_usage_core::alerts::step(
                        &alert_memory,
                        meta.provider,
                        &reading,
                        now,
                        &settings,
                    );
                    alert_memory = next_memory;
                    alerts.append(&mut step_alerts);
                }
                Err(err) => {
                    provider.state.attempted = true;
                    match err {
                        SourceError::NotConfigured(setup) => {
                            provider.state.last_failure =
                                Some((now, SourceError::NotConfigured(setup)));
                            meta.next_due = schedule_next(meta.trigger, now, visible);
                        }
                        SourceError::Unsupported => {
                            provider.state.last_failure = Some((now, SourceError::Unsupported));
                            meta.next_due = schedule_next(meta.trigger, now, visible);
                        }
                        SourceError::Unavailable(reason) => {
                            meta.failures = meta.failures.saturating_add(1);
                            meta.last_error = Some(reason);
                            provider.state.last_failure =
                                Some((now, SourceError::Unavailable(reason)));
                            let delay = Backoff::next_delay(meta.failures, reason, jitter.next());
                            meta.next_due = UnixMillis(now.0.saturating_add(delay_millis(delay)));
                            if matches!(meta.trigger, Trigger::Manual)
                                && provider.state.last_good.is_some()
                            {
                                provider.refresh.live_check_error = Some(reason);
                            }
                        }
                    }
                }
            }
        }

        if !alerts.is_empty() {
            let memory = alert_memory.clone();
            let store = Arc::clone(&store);
            let _ = tokio::task::spawn_blocking(move || store.save_alerts(&memory)).await;
            for alert in &alerts {
                notifier.notify(alert, settings.hour_cycle);
            }
        }

        emit_snapshot(&handle, &providers, &settings, clock.now());

        // Probe refresh: at most once per 6 hours when a needs_login source is due.
        let now = clock.now();
        if now.0.saturating_sub(last_probe_at.0) >= 6 * 3_600_000 {
            run_probe(&probe, &facts, &settings, &metas).await;
            last_probe_at = now;
        }
    }
}

fn recompute_live_check(
    metas: &[SourceMeta],
    providers: &mut [ProviderState],
    settings: &Settings,
) {
    for meta in metas {
        let slot = slot_of(meta.provider);
        if let Some(provider) = providers.get_mut(slot) {
            provider.refresh.free_refresh |= meta.cost == Cost::Free;
            provider.refresh.live_check =
                meta.cost == Cost::SpendsQuota && settings.claude_live_check;
        }
    }
}

async fn run_probe(
    probe: &Arc<dyn LoginProbe>,
    facts: &Arc<FactsCell>,
    settings: &Settings,
    metas: &[SourceMeta],
) {
    let mut wanted: Vec<ProviderKey> = metas
        .iter()
        .filter(|m| m.needs_login && !settings.hidden_providers.contains(&m.provider))
        .map(|m| m.provider)
        .collect();
    wanted.sort_by_key(|p| *p);
    wanted.dedup();
    if wanted.is_empty() {
        return;
    }
    let answers = probe.probe(wanted.clone()).await;
    let mut merged = facts.get();
    for p in wanted {
        merged = merged.with(p, answers.readiness(p), answers.plan(p));
    }
    facts.set(merged);
}

fn merge_reading(state: &mut SourceState, reading: &Reading) {
    let replace = match &state.last_good {
        None => true,
        Some(prev) => reading.observed_at.0 >= prev.observed_at.0,
    };
    if replace {
        state.last_good = Some(reading.clone());
    }
}

fn schedule_next(trigger: Trigger, now: UnixMillis, visible: bool) -> UnixMillis {
    match trigger {
        Trigger::OnSignal | Trigger::Manual => UnixMillis(u64::MAX),
        Trigger::Interval {
            visible: vis,
            hidden,
        } => {
            let d = if visible { vis } else { hidden };
            UnixMillis(now.0.saturating_add(delay_millis(d)))
        }
    }
}

fn next_deadline(metas: &[SourceMeta], providers: &[ProviderState], now: UnixMillis) -> UnixMillis {
    let mut next = u64::MAX;
    for m in metas {
        if !m.in_flight {
            next = next.min(m.next_due.0);
        }
    }
    for p in providers {
        if let Some(reading) = &p.state.last_good {
            let expires = reading
                .observed_at
                .0
                .saturating_add(delay_millis(p.fresh_for));
            next = next.min(expires);
        }
    }
    UnixMillis(if next > now.0 { next } else { now.0 })
}

async fn wait_until(timer: &Arc<dyn WallTimer>, deadline: UnixMillis) {
    let _ = timer.sleep_until(deadline).await;
}

async fn handle_refresh(
    clock: &Arc<dyn Clock>,
    settings: &Settings,
    target: RefreshTarget,
    metas: &mut [SourceMeta],
) -> Result<(), CommandError> {
    let now = clock.now();
    match target {
        RefreshTarget::All => {
            for meta in metas.iter_mut() {
                if meta.cost == Cost::Free && !settings.hidden_providers.contains(&meta.provider) {
                    meta.next_due = now;
                }
            }
            Ok(())
        }
        RefreshTarget::Provider { provider } => {
            if settings.hidden_providers.contains(&provider) {
                return Err(CommandError::NotEnabled);
            }
            let mut any = false;
            for meta in metas.iter_mut() {
                if meta.provider == provider && meta.cost == Cost::Free {
                    if meta.in_flight {
                        return Err(CommandError::Busy);
                    }
                    meta.next_due = now;
                    any = true;
                }
            }
            if any {
                Ok(())
            } else {
                Err(CommandError::NotEnabled)
            }
        }
        RefreshTarget::LiveCheck { provider } => {
            if !settings.claude_live_check {
                return Err(CommandError::NotEnabled);
            }
            for meta in metas.iter_mut() {
                if meta.provider == provider && meta.cost == Cost::SpendsQuota {
                    if meta.in_flight {
                        return Err(CommandError::Busy);
                    }
                    meta.next_due = now;
                    return Ok(());
                }
            }
            Err(CommandError::NotEnabled)
        }
    }
}

async fn handle_history(
    store: &Arc<dyn HistoryStore>,
    q: &HistoryQuery,
    now: UnixMillis,
) -> Result<HistorySeries, CommandError> {
    let span = match q.range {
        HistoryRange::Day => 86_400,
        HistoryRange::Week => 7 * 86_400,
        HistoryRange::Month => 30 * 86_400,
        HistoryRange::Quarter => 90 * 86_400,
    };
    let to = UnixSeconds(now.0 / 1000);
    let from = UnixSeconds(to.0.saturating_sub(span));
    let store = Arc::clone(store);
    let provider = q.provider;
    let meter = q.meter_id.clone();
    let data = tokio::task::spawn_blocking(move || store.series(provider, &meter, from, to, 600))
        .await
        .map_err(|_| CommandError::Internal)?
        .map_err(|_| CommandError::StoreUnavailable)?;
    let gap_s = data.bucket.as_secs();
    let points: Vec<HistoryPoint> = data
        .points
        .iter()
        .map(|p| HistoryPoint {
            t: p.at.0,
            v: p.value,
            lo: p.lo,
            hi: p.hi,
        })
        .collect();
    Ok(HistorySeries {
        provider: q.provider,
        meter_id: q.meter_id.clone(),
        from: from.0,
        to: to.0,
        unit: SeriesUnit::BasisPoints,
        points,
        gap_s,
        resets: data.resets.iter().map(|r| r.0).collect(),
    })
}

async fn handle_diagnostics(
    store: &Arc<dyn HistoryStore>,
    shell: &ShellFacts,
    metas: &[SourceMeta],
) -> Diagnostics {
    let store = Arc::clone(store);
    let stats = tokio::task::spawn_blocking(move || store.stats())
        .await
        .ok()
        .and_then(Result::ok);
    let (history_bytes, history_samples) = stats.map_or((0, 0), |s| (s.bytes, s.samples));
    Diagnostics {
        app_version: shell.app_version,
        sources: metas
            .iter()
            .map(|m| SourceDiag {
                source: m.id,
                provider: m.provider,
                last_ok_at: m.last_ok_at.map(|t| t.0),
                last_error: m.last_error,
                failures: m.failures,
            })
            .collect(),
        history_bytes,
        history_samples,
        tray_available: shell.tray_available,
    }
}

/// Recomputes the snapshot and sends it when its content differs from the last one.
fn emit_snapshot(
    handle: &ServiceHandle,
    providers: &[ProviderState],
    settings: &Settings,
    now: UnixMillis,
) {
    let usages: Vec<Usage> = providers
        .iter()
        .map(|p| {
            assess::derive(
                &p.state,
                now,
                &FreshnessPolicy {
                    fresh_for: p.fresh_for,
                },
            )
        })
        .collect();
    let states: Vec<view::ProviderState<'_>> = usages
        .iter()
        .zip(providers.iter())
        .enumerate()
        .map(|(i, (usage, provider))| view::ProviderState {
            provider: provider_at(i),
            usage,
            refresh: provider.refresh.clone(),
        })
        .collect();
    let snapshot = view::snapshot(&states, now, settings);
    if handle.snapshot.borrow().clone() != snapshot {
        let _ = handle.snapshot.send(snapshot);
    }
}

/// The retry schedule after failures (DESIGN 7.2). Pure.
#[derive(Copy, Clone, Debug)]
pub struct Backoff;

impl Backoff {
    /// The wait before the next attempt after `failures` consecutive failures (at least 1)
    /// of kind `kind`: `min(60 min, 1 min x 2^(failures-1))`, starting at 15 min for
    /// `RateLimited` and `Unauthorized`, with ±10 % from `jitter` in `0.0..1.0`.
    pub fn next_delay(failures: u32, kind: Unavailable, jitter: f64) -> Duration {
        let base: u64 = if matches!(kind, Unavailable::RateLimited | Unavailable::Unauthorized) {
            15
        } else {
            1
        };
        let factor = 2_u64.saturating_pow(failures.saturating_sub(1));
        let minutes = base.saturating_mul(factor).min(60);
        let jitter = if jitter.is_finite() {
            jitter.clamp(0.0, 1.0)
        } else {
            0.5
        };
        Duration::from_secs(minutes.saturating_mul(60)).mul_f64(0.9_f64.mul_add(1.0, jitter * 0.2))
    }
}

/// Jitter from `std::collections::hash_map::RandomState`: no `rand` dependency (DESIGN 8.9).
#[derive(Copy, Clone, Debug, Default)]
pub struct SystemJitter;

impl Jitter for SystemJitter {
    fn next(&self) -> f64 {
        use std::hash::{BuildHasher, Hasher};
        let hash = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        let upper = u32::try_from(hash >> 32).unwrap_or_default();
        f64::from(upper) / (f64::from(u32::MAX) + 1.0)
    }
}
