//! Desktop notifications (DESIGN 9.12, 8.8). The only module that may use `notify_rust`.
//! T15 writes the bodies.

use jev_usage_core::alerts::{Alert, AlertLevel};
use jev_usage_core::ipc::HourCycle;
use jev_usage_core::port::Notifier;
use jev_usage_core::text::LocalTime;
use jev_usage_core::time::{Clock, UnixSeconds};
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
        use notify_rust::Urgency;
        let urgency = if critical {
            Urgency::Critical
        } else {
            Urgency::Normal
        };
        notify_rust::Notification::new()
            .appname("Flashboard")
            .summary(title)
            .body(body)
            .urgency(urgency)
            .show()
            .is_ok()
    }
}

/// Builds the text in `core::text::notification` and hands it to its own worker thread.
pub struct DesktopNotifier {
    local: Arc<dyn LocalTime>,
    clock: Arc<dyn Clock>,
    sink: Arc<dyn NotificationSink>,
}

impl DesktopNotifier {
    /// A notifier over the given zone conversion, clock and sink.
    pub fn new(
        local: Arc<dyn LocalTime>,
        clock: Arc<dyn Clock>,
        sink: Arc<dyn NotificationSink>,
    ) -> Self {
        Self {
            local,
            clock,
            sink,
        }
    }
}

impl Notifier for DesktopNotifier {
    fn notify(&self, alert: &Alert, hour: HourCycle) {
        let now = UnixSeconds(self.clock.now().0 / 1000);
        let (title, body) =
            jev_usage_core::text::notification(alert, now, self.local.as_ref(), hour);
        let critical = matches!(alert.level, AlertLevel::Critical);
        let sink = Arc::clone(&self.sink);
        std::thread::spawn(move || {
            if !sink.deliver(&title, &body, critical) {
                jev_usage_io::diag::emit(jev_usage_io::diag::DiagEvent::NotifyFailed);
            }
        });
    }
}
