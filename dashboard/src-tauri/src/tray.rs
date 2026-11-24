//! The tray: menu rows and icon (DESIGN 9.11, 7.7). T11 writes the bodies.

use crate::tray_icon::{render, IconSpec};
use jev_usage_core::assess::Severity;
use jev_usage_core::ipc::{Settings, Snapshot};
use jev_usage_core::text::{LocalTime, TrayRow};
use jev_usage_core::time::UnixSeconds;
use std::sync::{Mutex, OnceLock};
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
    let rows = jev_usage_core::text::tray_rows(snapshot, now, local, settings.hour_cycle);
    let icon = match &snapshot.headline {
        Some(headline) => IconSpec {
            used_step: u8::try_from((headline.used_bp / 500).min(20)).unwrap_or(20),
            level: headline.severity,
        },
        None => IconSpec {
            used_step: 0,
            level: Severity::Unknown,
        },
    };
    TrayModel {
        rows,
        icon,
        notifications: settings.notifications,
    }
}

/// Creates the tray icon and its menu; id and title "Flashboard".
pub fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), TrayError> {
    let icon = render(
        IconSpec {
            used_step: 0,
            level: Severity::Unknown,
        },
        64,
    );
    let image = tauri::image::Image::new_owned(icon, 64, 64);
    let menu = tauri::menu::MenuBuilder::new(app)
        .build()
        .map_err(|_| TrayError::Unavailable)?;
    tauri::tray::TrayIconBuilder::with_id("flashboard")
        .icon(image)
        .tooltip("Flashboard")
        .menu(&menu)
        .build(app)
        .map_err(|_| TrayError::Unavailable)?;
    Ok(())
}

fn last_model() -> &'static Mutex<Option<TrayModel>> {
    static LAST: OnceLock<Mutex<Option<TrayModel>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

/// Touches the menu and icon only when `next` differs from the model last applied.
pub fn apply<R: Runtime>(app: &AppHandle<R>, next: TrayModel) {
    let changed = {
        let mut last = last_model().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *last == Some(next.clone()) {
            false
        } else {
            *last = Some(next.clone());
            true
        }
    };
    if !changed {
        return;
    }
    let icon = render(next.icon, 64);
    let image = tauri::image::Image::new_owned(icon, 64, 64);
    if let Some(tray) = app.tray_by_id("flashboard") {
        let _ = tray.set_icon(Some(image));
    }
    let _ = app;
}
