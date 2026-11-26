//! The manifests and lint policy of the three app crates (DESIGN 5 invariant 3, 8.9, 11.1).
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

fn dashboard() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manifests() -> [(String, String); 3] {
    [
        "crates/usage-core/Cargo.toml",
        "crates/usage-io/Cargo.toml",
        "src-tauri/Cargo.toml",
    ]
    .map(|path| {
        (
            path.to_owned(),
            std::fs::read_to_string(dashboard().join(path)).unwrap(),
        )
    })
}

/// The `[lints]` tables: everything from `[lints.rust]` to the end of the manifest.
fn lints_table(manifest: &str) -> &str {
    let at = manifest.find("[lints.rust]").expect("a [lints.rust] table");
    &manifest[at..]
}

#[test]
fn the_three_manifests_carry_an_identical_lints_table() {
    let [(_, core), (io_path, io), (shell_path, shell)] = manifests();
    let table = lints_table(&core);
    assert_eq!(table, lints_table(&io), "{io_path}");
    assert_eq!(table, lints_table(&shell), "{shell_path}");
    for needle in [
        "unsafe_code = \"forbid\"",
        "missing_docs = \"warn\"",
        "all = { level = \"deny\", priority = -1 }",
        "pedantic = { level = \"deny\", priority = -1 }",
        "unwrap_used = \"deny\"",
        "expect_used = \"deny\"",
        "panic = \"deny\"",
        "indexing_slicing = \"deny\"",
        "arithmetic_side_effects = \"deny\"",
        "as_conversions = \"deny\"",
        "print_stdout = \"deny\"",
        "print_stderr = \"deny\"",
        "dbg_macro = \"deny\"",
        "todo = \"deny\"",
        "unimplemented = \"deny\"",
        "disallowed_types = \"deny\"",
        "disallowed_macros = \"deny\"",
    ] {
        assert!(table.contains(needle), "the lints table lacks `{needle}`");
    }
}

#[test]
fn no_manifest_uses_workspace_lints_or_inherits_from_the_workspace() {
    for (path, text) in manifests() {
        assert!(!text.contains("lints.workspace"), "{path}");
        for line in text.lines() {
            let line = line.trim();
            assert!(
                !(line.starts_with("workspace") && line.contains("true")),
                "{path}: {line}"
            );
            assert!(!line.contains("workspace = true"), "{path}: {line}");
        }
        assert!(
            text.contains("publish = false"),
            "{path}: private crates only (cargo-deny, DESIGN 8.9)"
        );
        assert!(!text.contains("git ="), "{path}: no git sources");
    }
}

#[test]
fn every_registry_dependency_is_pinned_exactly() {
    for (path, text) in manifests() {
        let mut in_dependencies = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_dependencies = line.ends_with("dependencies]");
                continue;
            }
            if !in_dependencies || line.is_empty() || line.starts_with('#') {
                continue;
            }
            let pinned = line.contains("\"=") || line.contains("path = ");
            assert!(pinned, "{path}: not an exact pin: {line}");
        }
    }
}

#[test]
fn the_root_workspace_has_no_lint_table_and_no_panic_setting() {
    let root = std::fs::read_to_string(dashboard().join("../Cargo.toml")).unwrap();
    assert!(
        !root.contains("[workspace.lints"),
        "Flashboard never adds [workspace.lints]"
    );
    assert!(
        !root.lines().any(|l| l.trim_start().starts_with("panic")),
        "no profile sets `panic` (DESIGN 5)"
    );
    assert!(root.contains("[profile.dist]"));
    assert!(root.contains("exclude = [\".claude\", \"dashboard/fuzz\"]"));
    let defaults = root
        .lines()
        .find(|l| l.starts_with("default-members"))
        .unwrap();
    assert!(
        !defaults.contains("dashboard"),
        "the app crates are never default members"
    );
}

#[test]
fn the_clippy_configuration_is_the_dashboards_own() {
    assert!(dashboard().join("clippy.toml").is_file());
    assert!(
        !dashboard().join("../clippy.toml").exists(),
        "no clippy.toml at the repository root"
    );
    assert!(!dashboard().join("../.clippy.toml").exists());
    let text = std::fs::read_to_string(dashboard().join("clippy.toml")).unwrap();
    assert!(text.contains("serde_json::Value"));
    assert!(text.contains("serde_json::json"));
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n != "target" && n != "node_modules" && n != "gen")
            {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The fixed test header of DESIGN 11.1, without whitespace.
const TEST_HEADER: &str = "(clippy::unwrap_used,clippy::expect_used,clippy::panic,clippy::indexing_slicing,clippy::arithmetic_side_effects,clippy::as_conversions,clippy::disallowed_types,clippy::disallowed_macros)";
/// The one float cast of DESIGN 11.1, without whitespace.
const CAST: &str =
    "(clippy::as_conversions,clippy::cast_possible_truncation,clippy::cast_sign_loss)";

#[test]
fn the_only_lint_allows_are_the_test_header_and_the_one_cast() {
    let mut files = Vec::new();
    rust_files(&dashboard().join("crates"), &mut files);
    rust_files(&dashboard().join("src-tauri"), &mut files);
    assert!(files.len() > 40, "found only {} files", files.len());
    // Written in two halves so this file's own source does not contain the attribute name.
    let needles = [
        format!("[{}{}", "al", "low"),
        format!("[{}{}", "ex", "pect"),
    ];
    let mut casts = Vec::new();
    for path in &files {
        let compact: String = std::fs::read_to_string(path)
            .unwrap()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        for needle in &needles {
            let mut rest = compact.as_str();
            while let Some(at) = rest.find(needle.as_str()) {
                let after = &rest[at + needle.len()..];
                let end = after.find(")]").map_or(after.len(), |i| i + 1);
                let lints = after[..end].replace(",)", ")");
                if lints == CAST {
                    casts.push(path.clone());
                } else {
                    assert_eq!(
                        lints,
                        TEST_HEADER,
                        "{}: a lint attribute that is not approved",
                        path.display()
                    );
                }
                rest = &after[end..];
            }
        }
    }
    assert_eq!(casts.len(), 1, "exactly one float cast: {casts:?}");
    assert!(casts[0].ends_with("usage-core/src/quantity.rs"));
}

#[test]
fn every_crate_root_forbids_unsafe_code_on_its_first_line() {
    for root in [
        "crates/usage-core/src/lib.rs",
        "crates/usage-io/src/lib.rs",
        "src-tauri/src/lib.rs",
        "src-tauri/src/main.rs",
    ] {
        let text = std::fs::read_to_string(dashboard().join(root)).unwrap();
        assert_eq!(
            text.lines().next(),
            Some("#![forbid(unsafe_code)]"),
            "{root}"
        );
    }
}
