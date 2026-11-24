//! The wiring (DESIGN 8.8, TASKS T11). T00's scaffold: the single-instance plugin first,
//! the six commands, and the `main` window created in code and shown empty; T11 adds the
//! service, the tray, the signal listener and the window lifecycle.

use crate::{cli, commands, hardening};
use tauri::{WebviewUrl, WebviewWindowBuilder};

/// The label of the one window; the capability file grants it and nothing else.
pub const MAIN_WINDOW: &str = "main";
/// The product name: window title, tray tooltip, notifications (DESIGN 1a).
pub const PRODUCT_NAME: &str = "Flashboard";

/// Runs the app until it exits.
pub fn run() {
    hardening::apply();
    let args = cli::parse(std::env::args().skip(1));
    let outcome = tauri::Builder::default()
        // Registered first: a second process ends here, before anything else is created.
        // Its arguments and working directory are dropped, never stored or logged.
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {}))
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::refresh,
            commands::get_history,
            commands::get_settings,
            commands::update_settings,
            commands::get_diagnostics
        ])
        .setup(move |app| {
            if !args.hidden {
                WebviewWindowBuilder::new(app, MAIN_WINDOW, WebviewUrl::App("index.html".into()))
                    .title(PRODUCT_NAME)
                    .inner_size(960.0, 640.0)
                    .min_inner_size(360.0, 420.0)
                    .incognito(true)
                    .disable_drag_drop_handler()
                    .build()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!());
    if outcome.is_err() {
        std::process::exit(1);
    }
}
