//! The two events, Rust to UI only (DESIGN 7.3). T11 writes the body.

use jev_usage_core::ipc::Snapshot;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::watch;

/// The event that carries a changed `Snapshot`.
pub const EVENT_SNAPSHOT: &str = "snapshot";
/// The event that carries a `Route` from the tray menu.
pub const EVENT_NAVIGATE: &str = "navigate";

/// Emits every changed snapshot to the `main` window, only while it exists.
pub fn forward_snapshots<R: Runtime>(app: AppHandle<R>, mut rx: watch::Receiver<Snapshot>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if rx.changed().await.is_err() {
                return;
            }
            let snapshot = rx.borrow().clone();
            if let Some(window) = app.get_webview_window("main") {
                if window.is_visible().unwrap_or(false) {
                    let _ = window.emit(EVENT_SNAPSHOT, snapshot);
                }
            }
        }
    });
}
