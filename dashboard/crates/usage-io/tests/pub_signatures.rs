//! D11.1 / T12: no `std::io::Result` or `std::io::Error` in a `pub` signature. A public
//! function reports a closed, field-less error type (`FsError`, `Unavailable`, `StoreError`),
//! never an OS error that can carry a path or message.
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

/// The text of every public item head: from a line starting with `pub ` (not `pub use`/`mod`)
/// up to the first `{`, `;` or `=`-less line end, joined across lines for multi-line
/// signatures.
fn public_heads(source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut heads = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_start();
        if line.starts_with("pub ") || line.starts_with("pub(") {
            let mut head = String::new();
            let mut j = i;
            loop {
                head.push_str(lines[j]);
                head.push(' ');
                let done = lines[j].contains('{')
                    || lines[j].contains(';')
                    || lines[j].trim_end().ends_with(',');
                if done || j + 1 >= lines.len() || j > i + 40 {
                    break;
                }
                j += 1;
            }
            heads.push(head);
        }
        i += 1;
    }
    heads
}

#[test]
fn no_io_error_type_in_a_public_signature() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in [
        "crates/usage-core/src",
        "crates/usage-io/src",
        "src-tauri/src",
    ] {
        collect_rs(&root.join(dir), &mut files);
    }
    assert!(files.len() > 20, "scanned too few files: {}", files.len());
    let mut offenders = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap_or_default();
        // Stop at the unit-test module: test helpers may use anything.
        let source = source.split("#[cfg(test)]").next().unwrap_or("");
        for head in public_heads(source) {
            if head.contains("io::Result") || head.contains("io::Error") {
                offenders.push(format!("{}: {}", file.display(), head.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "std::io in public signatures: {offenders:#?}"
    );
}
