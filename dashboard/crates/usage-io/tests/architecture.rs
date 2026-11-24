//! The architecture test (DESIGN 8.10): forbidden patterns live only where the design says,
//! and the type/lint discipline holds. A plain Rust test over the dashboard's `.rs` files.
//!
//! Every needle is built from split string literals so this test file does not itself
//! contain the literal pattern (the other D8.10 tests scan the test files too).

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

use std::path::{Path, PathBuf};

fn dashboard_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is dashboard/crates/usage-io.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
}

fn src_files() -> Vec<PathBuf> {
    let root = dashboard_root();
    let mut files = Vec::new();
    for dir in ["crates/usage-core/src", "crates/usage-io/src", "src-tauri/src"] {
        collect_rs(&root.join(dir), &mut files);
    }
    files
}

fn rel(path: &Path) -> String {
    path.strip_prefix(dashboard_root())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn only_in(files: &[PathBuf], needle: &str, allowed: &[&str]) -> Vec<String> {
    let mut offenders = Vec::new();
    for file in files {
        let text = read(file);
        if text.contains(needle) {
            let r = rel(file);
            if !allowed.contains(&r.as_str()) {
                offenders.push(format!("{needle} in {r}"));
            }
        }
    }
    offenders
}

#[test]
fn each_forbidden_api_lives_only_in_its_one_file() {
    let files = src_files();
    let mut failures = Vec::new();
    // The one "exit" for a secret is defined in core and called only in io.
    let n_exit = format!("{}_{}(", "expose", "secret");
    let n_http = format!("{}{}", "req", "west");
    let n_proc = format!("{}::{}", "process", "Command");
    let n_bus = format!("{}{}", "zb", "us");
    let n_notify = format!("{}_{}", "notify", "rust");
    let n_probe = format!("{}_{}", "probe", "all");
    let n_path = format!("{}_{}_{}", "path", "from", "env");
    failures.extend(only_in(
        &files,
        &n_exit,
        &["crates/usage-core/src/secret.rs", "crates/usage-io/src/http.rs"],
    ));
    failures.extend(only_in(&files, &n_http, &["crates/usage-io/src/http.rs"]));
    failures.extend(only_in(
        &files,
        &n_proc,
        &["crates/usage-io/src/proc.rs"],
    ));
    failures.extend(only_in(&files, &n_bus, &["src-tauri/src/bus.rs"]));
    failures.extend(only_in(&files, &n_notify, &["src-tauri/src/notifier.rs"]));
    failures.extend(only_in(&files, &n_probe, &[]));
    failures.extend(only_in(&files, &n_path, &[]));
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_engine_is_named_only_in_the_bridge() {
    let files = src_files();
    let mut failures = Vec::new();
    for name in ["engine", "router", "embed"] {
        let needle = format!("{}_{}", "jev", name);
        failures.extend(only_in(
            &files,
            &needle,
            &["crates/usage-io/src/engine_bridge.rs"],
        ));
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn no_print_or_dbg_macros_in_source() {
    let files = src_files();
    let mut failures = Vec::new();
    for file in files {
        let text = read(&file);
        for macro_name in ["println", "eprintln", "dbg"] {
            let needle = format!("{macro_name}!");
            if text.contains(&needle) {
                failures.push(format!("{needle} in {}", rel(&file)));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn forbid_unsafe_code_is_the_first_line_of_every_crate_root() {
    let root = dashboard_root();
    let mut failures = Vec::new();
    for path in [
        "crates/usage-core/src/lib.rs",
        "crates/usage-io/src/lib.rs",
        "src-tauri/src/lib.rs",
    ] {
        let text = read(&root.join(path));
        let first = text.lines().next().unwrap_or("");
        if first != "#![forbid(unsafe_code)]" {
            failures.push(format!("{path}: first line is {first:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn no_serde_json_value_or_json_macro_outside_tests() {
    let files = src_files();
    let n_value = format!("{}::{}", "serde_json", "Value");
    let n_macro = format!("{}!", "json");
    let mut failures = Vec::new();
    for file in files {
        let text = read(&file);
        if text.contains(&n_value) || text.contains(&n_macro) {
            failures.push(rel(&file));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
