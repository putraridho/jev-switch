//! The tray: menu rows and icon (DESIGN 9.11, 7.7). T11 writes the bodies.

use crate::tray_icon::IconSpec;
use jev_usage_core::assess::Severity;
use jev_usage_core::ipc::{Settings, Snapshot};
use jev_usage_core::text::{LocalTime, TrayRow};
use jev_usage_core::time::UnixSeconds;
use tauri::{AppHandle, Runtime};

/// Everything the tray shows. The tray is touched only when this value changes.
#[derive(Clone, PartialEq, Debug)]
pub struct TrayModel {
    /// The menu's provider rows.
    pub rows: Vec<TrayRow>,
    /// The icon.
    pub icon: IconSpec,
    /// The state of the Notifications checkbox.
    pub notifications: bool,
}

/// Why the tray could not be installed.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum TrayError {
    /// The tray icon could not be created.
    Unavailable,
}

/// Pure: rows from `core::text::tray_rows`, icon from the headline.
pub fn model(
    snapshot: &Snapshot,
    settings: &Settings,
    now: UnixSeconds,
    local: &dyn LocalTime,
) -> TrayModel {
    let _ = (snapshot, now, local);
    TrayModel {
        rows: Vec::new(),
        icon: IconSpec {
            used_step: 0,
            level: Severity::Unknown,
        },
        notifications: settings.notifications,
    }
}

/// Creates the tray icon and its menu; id and title "Flashboard".
pub fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), TrayError> {
    let _ = app;
    Err(TrayError::Unavailable)
}

/// Touches the menu and icon only when `next` differs from the model last applied.
pub fn apply<R: Runtime>(app: &AppHandle<R>, next: TrayModel) {
    let _ = (app, next);
}
