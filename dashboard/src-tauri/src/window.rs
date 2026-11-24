//! The window's lifecycle (DESIGN 9.13, 8.8). T11 writes the bodies.

use crate::bus::TrayHost;
use jev_usage_core::ipc::Route;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// Creates the window at `route` (URL fragment: view, provider, theme; incognito; zoom;
/// drag-and-drop handler off), or shows the existing one and sends it `navigate`.
pub fn open<R: Runtime>(app: &AppHandle<R>, route: Route) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit(crate::events::EVENT_NAVIGATE, route);
        return;
    }
    let fragment = route_fragment(&route);
    let url = WebviewUrl::App(format!("index.html{fragment}").into());
    let _ = WebviewWindowBuilder::new(app, "main", url)
        .title("Flashboard")
        .inner_size(420.0, 640.0)
        .min_inner_size(320.0, 400.0)
        .incognito(true)
        .disable_drag_drop_handler()
        .build();
}

/// A tray host present now and `close_to_tray`: hide, destroy after 30 s; else quit.
pub fn close_requested<R: Runtime>(app: &AppHandle<R>, host: &dyn TrayHost) {
    if host.present() {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.hide();
        }
    } else {
        app.exit(0);
    }
}

/// Remembers the window size (debounced) in the store's meta table.
pub fn remember_size<R: Runtime>(app: &AppHandle<R>, w: u32, h: u32) {
    let _ = (app, w, h);
}

fn route_fragment(route: &Route) -> String {
    match route {
        Route::Overview => "#/overview".to_owned(),
        Route::Provider { .. } => "#/provider".to_owned(),
        Route::Settings => "#/settings".to_owned(),
    }
}
