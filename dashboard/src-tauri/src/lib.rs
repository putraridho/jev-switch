#![forbid(unsafe_code)]
//! Flashboard's Tauri shell (`docs/dashboard/DESIGN.md` 5.2): window, tray, notifications
//! and the six commands. Wiring only; every decision lives in `jev-usage-core` and every
//! adapter in `jev-usage-io`.

pub mod app;
pub mod autostart;
pub mod bus;
pub mod cli;
pub mod commands;
pub mod events;
pub mod hardening;
pub mod localtime;
pub mod notifier;
pub mod tray;
pub mod tray_icon;
pub mod window;

/// Starts Flashboard and returns when it exits.
pub fn run() {
    app::run();
}
