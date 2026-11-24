//! Drift guards and hardening proofs (DESIGN 8.8, 8.10): the capability file, the CSP, the
//! dependency features and the panic hook.

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

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

const CANARY: &str = "JEVCANARY_9f3c1b7e_DO_NOT_SURFACE";
const CHILD_VAR: &str = "JEV_PANIC_HOOK_CHILD";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn json(rel: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(root().join(rel)).unwrap()).unwrap()
}

#[test]
fn capabilities_are_exactly_the_event_listeners_and_six_commands() {
    let main = json("capabilities/main.json");
    assert_eq!(main["identifier"], "main");
    assert_eq!(main["windows"], serde_json::json!(["main"]));
    assert_eq!(main["local"], true);
    assert!(main.get("remote").is_none());
    let permissions: Vec<&str> = main["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert_eq!(
        permissions,
        [
            "core:event:allow-listen",
            "core:event:allow-unlisten",
            "allow-get-snapshot",
            "allow-refresh",
            "allow-get-history",
            "allow-get-settings",
            "allow-update-settings",
            "allow-get-diagnostics",
        ]
    );
    let files: Vec<_> = std::fs::read_dir(root().join("capabilities"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(files.len(), 1, "an extra capability file: {files:?}");
}

#[test]
fn csp_and_security_flags_are_frozen() {
    let conf = json("tauri.conf.json");
    let security = &conf["app"]["security"];
    assert_eq!(
        security["csp"],
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; \
         font-src 'self'; connect-src ipc: http://ipc.localhost; base-uri 'none'; \
         form-action 'none'; frame-ancestors 'none'; object-src 'none'"
    );
    assert_eq!(security["freezePrototype"], true);
    assert_eq!(security["dangerousDisableAssetCspModification"], false);
    assert_eq!(security["capabilities"], serde_json::json!(["main"]));
    assert_eq!(conf["app"]["withGlobalTauri"], false);
    assert_eq!(conf["app"]["windows"], serde_json::json!([]));
    assert_eq!(conf["identifier"], "dev.jev.flashboard");
}

/// The text of one dependency table of the manifest.
fn section(manifest: &str, name: &str) -> String {
    let header = format!("[{name}]");
    manifest
        .lines()
        .skip_while(|l| l.trim() != header)
        .skip(1)
        .take_while(|l| !l.trim_start().starts_with('['))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn plugin_list_is_one_and_no_devtools_feature() {
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).unwrap();
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let text = section(&manifest, table);
        let plugins: Vec<&str> = text
            .lines()
            .filter_map(|l| l.split('=').next())
            .map(str::trim)
            .filter(|name| name.starts_with("tauri-plugin"))
            .collect();
        let expected: &[&str] = if table == "dependencies" {
            &["tauri-plugin-single-instance"]
        } else {
            &[]
        };
        assert_eq!(plugins, expected, "[{table}]");
        for line in text.lines().filter(|l| l.trim_start().starts_with("tauri")) {
            assert!(!line.contains("devtools"), "{line}");
        }
    }
    assert!(!manifest.contains("devtools"));
}

/// The child half of the panic-hook proof: a no-op unless the parent started it.
#[test]
fn panic_hook_child() {
    if std::env::var_os(CHILD_VAR).is_none() {
        return;
    }
    jev_dashboard::hardening::apply();
    panic!("{CANARY} /home/someone/secret/path");
}

#[test]
fn panic_hook_prints_a_base_name_and_a_line_and_no_path_or_payload() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "panic_hook_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_VAR, "1")
        .output()
        .unwrap();
    assert!(!output.status.success(), "the child must have panicked");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stderr
        .lines()
        .find(|l| l.starts_with("panicked at "))
        .unwrap_or_else(|| panic!("no hook line in stderr: {stderr:?}"));
    let location = line.trim_start_matches("panicked at ");
    let (file, number) = location.rsplit_once(':').unwrap();
    assert_eq!(file, "hardening.rs");
    assert!(number.parse::<u32>().is_ok());
    for stream in [stderr.as_ref(), stdout.as_ref()] {
        assert!(!stream.contains(CANARY), "{stream:?}");
        assert!(!stream.contains("/home/someone"), "{stream:?}");
        assert!(!stream.contains('/'), "a path separator: {stream:?}");
    }
}
