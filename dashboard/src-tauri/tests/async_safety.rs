//! Regressions found by the T12 security review: the shell must not panic or loosen a rule
//! when called the way the app calls it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use jev_dashboard::autostart::{reconcile, AutostartError};
use jev_dashboard::bus::{SessionBusTrayHost, TrayHost};
use jev_dashboard::window::automation_flag;
use std::path::PathBuf;

/// `get_diagnostics` is an async command and asks the tray host from inside the runtime.
/// The bus call builds its own runtime, which panics on a thread that is already driving one.
#[tokio::test]
async fn tray_host_can_be_asked_from_inside_a_runtime() {
    let _present: bool = SessionBusTrayHost.present();
}

#[test]
fn only_the_exact_value_true_turns_automation_on() {
    assert!(automation_flag(Some("true")));
    for other in [
        None,
        Some(""),
        Some("1"),
        Some("TRUE"),
        Some("True"),
        Some("false"),
        Some(" true"),
    ] {
        assert!(!automation_flag(other), "{other:?}");
    }
}

#[test]
fn a_line_break_in_the_exec_path_writes_nothing() {
    let root = std::env::temp_dir().join(format!("flashboard-autostart-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let exec = root.join("evil\nExec=sh");
    std::fs::write(&exec, b"#!/bin/sh\n").unwrap();
    let config = root.join("config");
    let result = reconcile(true, &exec, &config);
    let wrote = config.join("autostart").exists();
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(result, Err(AutostartError::InvalidExec));
    assert!(!wrote);
}

/// `test-support` (plain HTTP, fakes) is for tests only: in the shell's manifest it may appear
/// only under `[dev-dependencies]`, so no release build can enable it.
#[test]
fn test_support_is_enabled_only_by_dev_dependencies() {
    let manifest =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .unwrap();
    let mut section = String::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed.to_owned();
        } else if trimmed.contains("test-support") {
            assert_eq!(
                section, "[dev-dependencies]",
                "test-support in {section}: {trimmed}"
            );
        }
    }
    let io = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates/usage-io/Cargo.toml"),
    )
    .unwrap();
    let default_line = io
        .lines()
        .find(|l| l.trim_start().starts_with("default"))
        .unwrap_or("");
    assert!(
        !default_line.contains("test-support"),
        "test-support must not be a default feature"
    );
}
