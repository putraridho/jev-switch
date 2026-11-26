//! T12 IPC audit (DESIGN 8.10): the key allow-list and the string-leaf vocabulary over
//! generated values, and the allow-list over the generated `.ts` files.
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

use jev_usage_core::ipc::{
    Currency, HistoryPoint, LimitStatus, MeterView, Phase, Severity, WindowKind,
};
use jev_usage_core::label::Label;
use jev_usage_core::meter::MeterId;
use proptest::prelude::*;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn dashboard() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn allowlist() -> BTreeSet<String> {
    std::fs::read_to_string(dashboard().join("fixtures/ipc-allowlist.txt"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn walk(value: &Value, keys: &mut BTreeSet<String>, strings: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                keys.insert(key.clone());
                walk(inner, keys, strings);
            }
        }
        Value::Array(items) => items.iter().for_each(|inner| walk(inner, keys, strings)),
        Value::String(s) => strings.push(s.clone()),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn closed_shape(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 48
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_:.+-".contains(&b))
}

fn currency() -> impl Strategy<Value = Currency> {
    prop_oneof![Just(Currency::Cny), Just(Currency::Other)]
}

fn window() -> impl Strategy<Value = WindowKind> {
    prop_oneof![
        Just(WindowKind::FiveHour),
        (1u32..100_000).prop_map(|minutes| WindowKind::Rolling { minutes }),
        "[a-z0-9_]{1,12}".prop_map(|s| WindowKind::Named {
            label: Label::parse(&s).unwrap()
        }),
    ]
}

fn severity() -> impl Strategy<Value = Severity> {
    prop_oneof![Just(Severity::Ok), Just(Severity::Critical)]
}

fn meter_view() -> impl Strategy<Value = MeterView> {
    prop_oneof![
        (
            window(),
            0u16..=10_000,
            proptest::option::of(any::<u32>().prop_map(u64::from)),
            severity()
        )
            .prop_map(|(window, used_bp, resets_at, severity)| MeterView::Window {
                id: MeterId::Window(window.clone()),
                window,
                used_bp,
                resets_at,
                status: LimitStatus::Allowed,
                severity,
                phase: Phase::Current,
            }),
        (
            currency(),
            any::<i64>(),
            proptest::option::of(any::<i64>()),
            any::<bool>()
        )
            .prop_map(
                |(currency, total_micros, parts, usable)| MeterView::Balance {
                    id: MeterId::Balance(currency),
                    currency,
                    total_micros,
                    granted_micros: parts,
                    topped_up_micros: parts,
                    usable,
                    severity: Severity::Ok,
                }
            ),
    ]
}

proptest! {
    #[test]
    fn generated_meter_views_have_only_allowed_keys_and_closed_strings(view in meter_view()) {
        let allowed = allowlist();
        let value = serde_json::to_value(&view).unwrap();
        let (mut keys, mut strings) = (BTreeSet::new(), Vec::new());
        walk(&value, &mut keys, &mut strings);
        prop_assert!(keys.is_subset(&allowed), "keys: {:?}", keys.difference(&allowed).collect::<Vec<_>>());
        for s in &strings {
            prop_assert!(closed_shape(s), "string leaf {s:?}");
        }
    }
}

/// Every property name in every generated `.ts` file is on the allow-list. Names are the
/// identifiers directly before a `:` at the top level of a type body; doc comments are
/// stripped first.
#[test]
fn generated_typescript_declares_only_allowed_keys() {
    let allowed = allowlist();
    let dir = dashboard().join("src/lib/ipc/bindings");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let mut stripped = String::new();
        let mut rest = text.as_str();
        while let Some(start) = rest.find("/*") {
            stripped.push_str(&rest[..start]);
            let end = rest[start..]
                .find("*/")
                .map_or(rest.len(), |e| start + e + 2);
            rest = &rest[end..];
        }
        stripped.push_str(rest);
        for line in stripped
            .lines()
            .filter(|l| !l.starts_with("import") && !l.starts_with("//"))
        {
            let bytes = line.as_bytes();
            for (i, &b) in bytes.iter().enumerate() {
                if b != b':' {
                    continue;
                }
                // Skip the `: ` of `"kind": "window"` style quoted names too: take the
                // identifier (optionally quoted) immediately before the colon.
                let before = line[..i].trim_end_matches('"');
                let name: String = before
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                if name.is_empty() {
                    continue;
                }
                seen += 1;
                assert!(
                    allowed.contains(&name),
                    "{}: property {name:?} is not on the allow-list",
                    path.display()
                );
            }
        }
    }
    assert!(seen > 40, "scanned suspiciously few properties: {seen}");
}

#[test]
fn history_point_is_four_numbers() {
    let value = serde_json::to_value(HistoryPoint {
        t: 1,
        v: 2,
        lo: 2,
        hi: 2,
    })
    .unwrap();
    let mut keys = BTreeSet::new();
    walk(&value, &mut keys, &mut Vec::new());
    assert_eq!(
        keys,
        ["t", "v", "lo", "hi"]
            .map(str::to_owned)
            .into_iter()
            .collect::<BTreeSet<_>>()
    );
}
