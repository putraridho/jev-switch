//! Desktop notifications (DESIGN 9.12, 8.8). The only module that may use `notify_rust`.
//! T15 writes the bodies.

use jev_usage_core::alerts::Alert;
use jev_usage_core::ipc::HourCycle;
use jev_usage_core::port::Notifier;
use jev_usage_core::text::LocalTime;
use jev_usage_core::time::Clock;
use std::sync::Arc;

/// The seam to the notification library, so the notifier is testable without a bus.
pub trait NotificationSink: Send + Sync {
    /// Shows one notification; false when it could not be delivered.
    fn deliver(&self, title: &str, body: &str, critical: bool) -> bool;
}

/// The production sink: app name Flashboard, the desktop-entry hint, urgency by level.
#[derive(Copy, Clone, Debug, Default)]
pub struct NotifyRustSink;

impl NotificationSink for NotifyRustSink {
    fn deliver(&self, title: &str, body: &str, critical: bool) -> bool {
        let _ = (title, body, critical);
        false
    }
}

/// Builds the text in `core::text::notification` and hands it to its own worker thread.
pub struct DesktopNotifier {
    _local: Arc<dyn LocalTime>,
    _clock: Arc<dyn Clock>,
    _sink: Arc<dyn NotificationSink>,
}

impl DesktopNotifier {
    /// A notifier over the given zone conversion, clock and sink.
    pub fn new(
        local: Arc<dyn LocalTime>,
        clock: Arc<dyn Clock>,
        sink: Arc<dyn NotificationSink>,
    ) -> Self {
        Self {
            _local: local,
            _clock: clock,
            _sink: sink,
        }
    }
}

impl Notifier for DesktopNotifier {
    fn notify(&self, alert: &Alert, hour: HourCycle) {
        let _ = (alert, hour);
    }
}
