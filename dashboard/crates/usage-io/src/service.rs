//! The usage service: one task that owns all mutable state (DESIGN 7.1, 7.2). The types
//! are frozen; T08 writes the bodies.

use crate::probe::{FactsCell, LoginProbe};
use crate::timer::WallTimer;
use jev_usage_core::ipc::{
    CommandError, Diagnostics, HistoryQuery, HistorySeries, RefreshTarget, Settings, SettingsPatch,
    Snapshot,
};
use jev_usage_core::port::{
    HistoryStore, Notifier, SettingsStore, SourceId, StoreError, UsageSource,
};
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::time::Clock;
use jev_usage_core::usage::Unavailable;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

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

/// The handle the shell holds. Cloneable; dropping every clone stops the task.
#[derive(Clone, Debug)]
pub struct ServiceHandle {
    snapshot: watch::Sender<Snapshot>,
    settings: watch::Sender<Settings>,
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
        let _ = target;
        Err(CommandError::Internal)
    }

    /// One meter's history over a range: at most 600 points.
    pub async fn history(&self, q: HistoryQuery) -> Result<HistorySeries, CommandError> {
        let _ = q;
        Err(CommandError::Internal)
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
        let _ = patch;
        Err(CommandError::Internal)
    }

    /// Tells the service whether the window is visible (it polls less while hidden).
    pub fn set_visible(&self, visible: bool) {
        let _ = visible;
    }

    /// Version, per-source state and store size; reads store statistics off the loop.
    pub async fn diagnostics(&self, shell: ShellFacts) -> Diagnostics {
        Diagnostics {
            app_version: shell.app_version,
            sources: Vec::new(),
            history_bytes: 0,
            history_samples: 0,
            tray_available: shell.tray_available,
        }
    }

    /// Cancels in-flight fetches (killing their process groups) and stops the task.
    /// Bounded: 2 s.
    pub async fn shutdown(&self) {}
}

/// Starts the service task. `rt` is the runtime the task runs on: the shell passes Tauri's,
/// because Tauri's `setup` hook is not inside a runtime and a bare `tokio::spawn` would panic.
pub fn spawn(deps: ServiceDeps, rt: &tokio::runtime::Handle) -> ServiceHandle {
    let _ = (deps, rt);
    let (snapshot, _) = watch::channel(Snapshot {
        generated_at: 0,
        providers: Vec::new(),
        headline: None,
    });
    let (settings, _) = watch::channel(Settings::defaults());
    ServiceHandle { snapshot, settings }
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
