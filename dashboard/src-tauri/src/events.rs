//! The two events, Rust to UI only (DESIGN 7.3). T11 writes the body.

use jev_chat_core::ipc::ChatEvent;
use jev_usage_core::ipc::Snapshot;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::{broadcast, watch};

/// The event that carries a changed `Snapshot`.
pub const EVENT_SNAPSHOT: &str = "snapshot";
/// The event that carries a `Route` from the tray menu.
pub const EVENT_NAVIGATE: &str = "navigate";
/// The event that carries live coding-agent chat updates.
pub const EVENT_CHAT: &str = "chat_event";

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

/// Emits live coding-agent chat events to the `main` window while it is open.
pub fn forward_chat_events<R: Runtime>(app: AppHandle<R>, mut rx: broadcast::Receiver<ChatEvent>) {
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Some(window) = app.get_webview_window("main") {
                        if window.is_visible().unwrap_or(false) {
                            let _ = window.emit(EVENT_CHAT, event);
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}
