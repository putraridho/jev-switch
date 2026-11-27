//! The window's lifecycle (DESIGN 9.13, 8.8). T11 writes the bodies.

use crate::bus::TrayHost;
use jev_usage_core::ipc::Route;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// When the current web view was created, until its first snapshot has been served.
static CREATED_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Called by `get_snapshot`: the first call after a window was created emits `WindowReady`
/// with the milliseconds since creation (DESIGN 11.4); later calls do nothing.
pub fn first_snapshot_served() {
    let Ok(mut created) = CREATED_AT.lock() else {
        return;
    };
    if let Some(began) = created.take() {
        jev_usage_io::diag::emit(jev_usage_io::diag::DiagEvent::WindowReady {
            took_ms: u32::try_from(began.elapsed().as_millis()).unwrap_or(u32::MAX),
        });
    }
}

/// True only when `tauri-driver` launched the app for the WebDriver end-to-end test. wry gives an
/// incognito view its own ephemeral context, which cannot be automated (D16, checked in
/// T12), so that one test runs the view non-incognito inside its temp tree. Normal launches
/// never have this variable and stay incognito.
fn automation_requested() -> bool {
    automation_flag(std::env::var("TAURI_WEBVIEW_AUTOMATION").ok().as_deref())
}

/// The variable counts only when it is exactly `true` (what `tauri-driver` sets); anything
/// else, including `1`, `TRUE` or an empty value, leaves the view incognito.
pub fn automation_flag(value: Option<&str>) -> bool {
    value == Some("true")
}

/// Creates the window at `route` (URL fragment: view, provider, theme; incognito; zoom;
/// drag-and-drop handler off), or shows the existing one and sends it `navigate`.
pub fn open<R: Runtime>(app: &AppHandle<R>, route: Route) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit(crate::events::EVENT_NAVIGATE, route);
        return;
    }
    if let Ok(mut created) = CREATED_AT.lock() {
        *created = Some(std::time::Instant::now());
    }
    let fragment = route_fragment(&route);
    let url = WebviewUrl::App(format!("index.html{fragment}").into());
    let _ = WebviewWindowBuilder::new(app, "main", url)
        .title("Flashboard")
        .inner_size(560.0, 720.0)
        .min_inner_size(320.0, 400.0)
        .incognito(!automation_requested())
        .disable_drag_drop_handler()
        .build();
    if let Some(svc) = app.try_state::<jev_usage_io::service::ServiceHandle>() {
        apply_zoom(app, svc.settings().zoom_pct);
    }
}

/// Applies the zoom setting (100 to 200 %) to the whole web view (DESIGN 9.10, 8.8).
pub fn apply_zoom<R: Runtime>(app: &AppHandle<R>, zoom_pct: u16) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_zoom(f64::from(zoom_pct) / 100.0);
    }
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
        Route::Chat => "#/chat".to_owned(),
        Route::Provider { .. } => "#/provider".to_owned(),
        Route::Settings => "#/settings".to_owned(),
    }
}
