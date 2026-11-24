//! The window's lifecycle (DESIGN 9.13, 8.8). T11 writes the bodies.

use crate::bus::TrayHost;
use jev_usage_core::ipc::Route;
use tauri::{AppHandle, Runtime};

/// Creates the window at `route` (URL fragment: view, provider, theme; incognito; zoom;
/// drag-and-drop handler off), or shows the existing one and sends it `navigate`.
pub fn open<R: Runtime>(app: &AppHandle<R>, route: Route) {
    let _ = (app, route);
}

/// A tray host present now and `close_to_tray`: hide, destroy after 30 s; else quit.
pub fn close_requested<R: Runtime>(app: &AppHandle<R>, host: &dyn TrayHost) {
    let _ = (app, host.present());
}

/// Remembers the window size (debounced) in the store's meta table.
pub fn remember_size<R: Runtime>(app: &AppHandle<R>, w: u32, h: u32) {
    let _ = (app, w, h);
}
