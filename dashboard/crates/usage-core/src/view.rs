//! Domain to IPC read model (DESIGN 7.4). T02 writes the function bodies.

use crate::ipc::{RefreshView, Snapshot};
use crate::provider::ProviderKey;
use crate::settings::Settings;
use crate::time::UnixMillis;
use crate::usage::Usage;

/// One provider as the service holds it, ready to be turned into its view.
#[derive(Clone, PartialEq, Debug)]
pub struct ProviderState<'a> {
    /// The provider.
    pub provider: ProviderKey,
    /// Its derived state.
    pub usage: &'a Usage,
    /// Its refresh bookkeeping.
    pub refresh: RefreshView,
}

fn reading_view(
    reading: &crate::meter::Reading,
    now: UnixMillis,
    settings: &Settings,
) -> crate::ipc::ReadingView {
    let meters = reading
        .meters
        .as_slice()
        .iter()
        .map(|meter| {
            let severity = crate::assess::severity(meter, now, &settings.thresholds());
            match meter {
                crate::meter::Meter::Window(w) => crate::ipc::MeterView::Window {
                    id: meter.id(),
                    window: w.kind.clone(),
                    used_bp: w.used.basis_points(),
                    resets_at: w.resets_at.map(|v| v.0),
                    status: w.status,
                    severity,
                    phase: crate::assess::phase(w, now),
                },
                crate::meter::Meter::Balance(b) => crate::ipc::MeterView::Balance {
                    id: meter.id(),
                    currency: b.currency,
                    total_micros: b.total.micros(),
                    granted_micros: b.granted.map(crate::quantity::Money::micros),
                    topped_up_micros: b.topped_up.map(crate::quantity::Money::micros),
                    usable: b.usable,
                    severity,
                },
            }
        })
        .collect();
    crate::ipc::ReadingView {
        observed_at: reading.observed_at.0,
        plan: reading.plan.clone(),
        meters,
    }
}

/// The whole read model the UI and the tray render from.
pub fn snapshot(states: &[ProviderState<'_>], now: UnixMillis, settings: &Settings) -> Snapshot {
    let mut providers: Vec<_> = states
        .iter()
        .map(|state| {
            let usage = match state.usage {
                Usage::Pending => crate::ipc::UsageView::Pending,
                Usage::Known(r) => crate::ipc::UsageView::Known {
                    reading: reading_view(r, now, settings),
                },
                Usage::Stale { last, since, cause } => crate::ipc::UsageView::Stale {
                    reading: reading_view(last, now, settings),
                    since: since.0,
                    cause: *cause,
                },
                Usage::Unavailable { reason } => {
                    crate::ipc::UsageView::Unavailable { reason: *reason }
                }
                Usage::NotConfigured { setup } => {
                    crate::ipc::UsageView::NotConfigured { setup: *setup }
                }
                Usage::Unsupported => crate::ipc::UsageView::Unsupported,
            };
            crate::ipc::ProviderView {
                provider: state.provider,
                enabled: !settings.hidden_providers.contains(&state.provider),
                usage,
                refresh: state.refresh.clone(),
            }
        })
        .collect();
    providers.sort_by_key(|p| p.provider);
    let inputs: Vec<_> = states.iter().map(|s| (s.provider, s.usage)).collect();
    let headline = crate::summary::headline(&inputs, now, settings).map(|h| crate::ipc::Headline {
        provider: h.provider,
        meter_id: h.meter,
        used_bp: h.used.basis_points(),
        severity: h.severity,
        resets_at: h.resets_at.map(|v| v.0),
    });
    Snapshot {
        generated_at: now.0,
        providers,
        headline,
    }
}

/// Whether two snapshots show the same thing; ignores `generated_at`.
pub fn same_content(a: &Snapshot, b: &Snapshot) -> bool {
    a.providers == b.providers && a.headline == b.headline
}
