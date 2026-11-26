//! Every IPC type against DESIGN 7.4: wire names, tags, key allow-list and the generated bindings.
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
    CommandError, Currency, Diagnostics, Headline, HistoryPoint, HistoryQuery, HistoryRange,
    HistorySeries, HourCycle, LimitStatus, MeterView, Outcome, Phase, ProviderKey, ProviderView,
    ReadingView, RefreshTarget, RefreshView, Route, SeriesUnit, Settings, SettingsPatch, Setup,
    Severity, Snapshot, SourceDiag, SourceId, StaleCause, Theme, Unavailable, UsageView,
    WindowKind,
};
use jev_usage_core::label::Label;
use jev_usage_core::meter::MeterId;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap()
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn label(s: &str) -> Label {
    Label::parse(s).unwrap()
}

fn window_meter(resets_at: Option<u64>) -> MeterView {
    MeterView::Window {
        id: MeterId::Window(WindowKind::FiveHour),
        window: WindowKind::FiveHour,
        used_bp: 5_000,
        resets_at,
        status: LimitStatus::Allowed,
        severity: Severity::Ok,
        phase: Phase::Current,
    }
}

fn balance_meter(parts: Option<i64>) -> MeterView {
    MeterView::Balance {
        id: MeterId::Balance(Currency::Cny),
        currency: Currency::Cny,
        total_micros: 110_000_000,
        granted_micros: parts,
        topped_up_micros: parts,
        usable: true,
        severity: Severity::Ok,
    }
}

fn reading(plan: Option<Label>, some: bool) -> ReadingView {
    ReadingView {
        observed_at: 1_794_890_000_000,
        plan,
        meters: vec![
            window_meter(some.then_some(1_794_898_800)),
            balance_meter(some.then_some(10_000_000)),
        ],
    }
}

fn refresh(some: bool) -> RefreshView {
    RefreshView {
        in_flight: false,
        last_attempt_at: some.then_some(1_794_890_000_000),
        next_at: some.then_some(1_794_890_300_000),
        free_refresh: true,
        live_check: false,
        live_check_error: some.then_some(Unavailable::Timeout),
    }
}

/// One snapshot that holds every `UsageView` state; `some` fills or empties every optional field.
fn snapshot(some: bool) -> Snapshot {
    let plan = some.then(|| label("max_5x"));
    let usages = [
        UsageView::Known {
            reading: reading(plan.clone(), some),
        },
        UsageView::Stale {
            reading: reading(plan, some),
            since: 1_794_890_600_000,
            cause: StaleCause::Failing {
                reason: Unavailable::Offline,
            },
        },
        UsageView::NotConfigured {
            setup: Setup::AddDeepSeekKey,
        },
        UsageView::Unsupported,
        UsageView::Pending,
        UsageView::Unavailable {
            reason: Unavailable::NotInstalled,
        },
    ];
    Snapshot {
        generated_at: 1_794_890_700_000,
        providers: ProviderKey::ALL
            .into_iter()
            .zip(usages)
            .map(|(provider, usage)| ProviderView {
                provider,
                enabled: true,
                usage,
                refresh: refresh(some),
            })
            .collect(),
        headline: some.then_some(Headline {
            provider: ProviderKey::Claude,
            meter_id: MeterId::Window(WindowKind::FiveHour),
            used_bp: 5_000,
            severity: Severity::Ok,
            resets_at: Some(1_794_898_800),
        }),
    }
}

fn headline_without_reset() -> Headline {
    Headline {
        provider: ProviderKey::Codex,
        meter_id: MeterId::Window(WindowKind::Monthly),
        used_bp: 400,
        severity: Severity::Ok,
        resets_at: None,
    }
}

fn series() -> HistorySeries {
    HistorySeries {
        provider: ProviderKey::Claude,
        meter_id: MeterId::Window(WindowKind::SevenDay),
        from: 1_794_800_000,
        to: 1_794_886_400,
        unit: SeriesUnit::BasisPoints,
        points: vec![HistoryPoint {
            t: 1_794_800_000,
            v: 3_700,
            lo: 3_600,
            hi: 3_700,
        }],
        gap_s: 4_500,
        resets: vec![1_794_850_000],
    }
}

fn diagnostics(some: bool) -> Diagnostics {
    Diagnostics {
        app_version: "0.1.0",
        sources: vec![SourceDiag {
            source: SourceId::DeepSeekBalance,
            provider: ProviderKey::DeepSeek,
            last_ok_at: some.then_some(1_794_890_000_000),
            last_error: some.then_some(Unavailable::Unauthorized),
            failures: 2,
        }],
        history_bytes: 4_096,
        history_samples: 12,
        tray_available: true,
    }
}

#[test]
fn every_enum_variant_serialises_to_its_documented_form() {
    // Plain enums: the snake_case name.
    for (value, name) in [
        (Severity::Ok, "ok"),
        (Severity::Warn, "warn"),
        (Severity::Critical, "critical"),
        (Severity::Unknown, "unknown"),
    ] {
        assert_eq!(json(&value), format!("\"{name}\""));
    }
    for (value, name) in [(Phase::Current, "current"), (Phase::Elapsed, "elapsed")] {
        assert_eq!(json(&value), format!("\"{name}\""));
    }
    for (value, name) in [
        (LimitStatus::Allowed, "allowed"),
        (LimitStatus::Warning, "warning"),
        (LimitStatus::Rejected, "rejected"),
        (LimitStatus::Unknown, "unknown"),
    ] {
        assert_eq!(json(&value), format!("\"{name}\""));
    }
    for (value, name) in [
        (Currency::Usd, "usd"),
        (Currency::Cny, "cny"),
        (Currency::Other, "other"),
    ] {
        assert_eq!(json(&value), format!("\"{name}\""));
    }
    for (value, name) in [
        (HistoryRange::Day, "day"),
        (HistoryRange::Week, "week"),
        (HistoryRange::Month, "month"),
        (HistoryRange::Quarter, "quarter"),
    ] {
        assert_eq!(json(&value), format!("\"{name}\""));
        assert_eq!(
            serde_json::from_str::<HistoryRange>(&format!("\"{name}\"")).unwrap(),
            value
        );
    }
    for (value, name) in [
        (SeriesUnit::BasisPoints, "basis_points"),
        (SeriesUnit::Micros, "micros"),
    ] {
        assert_eq!(json(&value), format!("\"{name}\""));
    }
    for (value, name) in [
        (Theme::System, "system"),
        (Theme::Light, "light"),
        (Theme::Dark, "dark"),
    ] {
        assert_eq!(json(&value), format!("\"{name}\""));
        assert_eq!(
            serde_json::from_str::<Theme>(&format!("\"{name}\"")).unwrap(),
            value
        );
    }
    for (value, name) in [
        (HourCycle::Auto, "auto"),
        (HourCycle::H12, "h12"),
        (HourCycle::H24, "h24"),
    ] {
        assert_eq!(json(&value), format!("\"{name}\""));
        assert_eq!(
            serde_json::from_str::<HourCycle>(&format!("\"{name}\"")).unwrap(),
            value
        );
    }
    for (value, name) in [
        (Setup::UseGateway, "use_gateway"),
        (Setup::AddDeepSeekKey, "add_deepseek_key"),
    ] {
        assert_eq!(json(&value), format!("\"{name}\""));
    }
    for (value, name) in [
        (SourceId::ClaudeGatewayLog, "claude_gateway_log"),
        (SourceId::ClaudeLive, "claude_live"),
        (SourceId::CodexAppServer, "codex_app_server"),
        (SourceId::DeepSeekBalance, "deepseek_balance"),
        (SourceId::GeminiNone, "gemini_none"),
        (SourceId::AntigravityCli, "antigravity_cli"),
        (SourceId::GrokNone, "grok_none"),
    ] {
        assert_eq!(json(&value), format!("\"{name}\""));
    }
    for (value, name) in [
        (Unavailable::NotInstalled, "not_installed"),
        (Unavailable::NotLoggedIn, "not_logged_in"),
        (Unavailable::Offline, "offline"),
        (Unavailable::Timeout, "timeout"),
        (Unavailable::RateLimited, "rate_limited"),
        (Unavailable::Unauthorized, "unauthorized"),
        (Unavailable::ServerError, "server_error"),
        (Unavailable::Protocol, "protocol"),
        (Unavailable::Io, "io"),
        (Unavailable::Internal, "internal"),
    ] {
        assert_eq!(json(&value), format!("\"{name}\""));
    }
    for p in ProviderKey::ALL {
        assert_eq!(json(&p), format!("\"{}\"", p.as_str()));
    }

    // Internally tagged enums: where a tagged newtype variant would fail at run time.
    assert_eq!(json(&StaleCause::Quiet), r#"{"kind":"quiet"}"#);
    assert_eq!(
        json(&StaleCause::Failing {
            reason: Unavailable::Io
        }),
        r#"{"kind":"failing","reason":"io"}"#
    );
    assert_eq!(json(&WindowKind::FiveHour), r#"{"kind":"five_hour"}"#);
    assert_eq!(json(&WindowKind::SevenDay), r#"{"kind":"seven_day"}"#);
    assert_eq!(json(&WindowKind::Monthly), r#"{"kind":"monthly"}"#);
    assert_eq!(
        json(&WindowKind::Rolling { minutes: 90 }),
        r#"{"kind":"rolling","minutes":90}"#
    );
    assert_eq!(
        json(&WindowKind::Named {
            label: label("7d_oi")
        }),
        r#"{"kind":"named","label":"7d_oi"}"#
    );
    assert_eq!(
        json(&window_meter(Some(1_794_898_800))),
        r#"{"kind":"window","id":"w:5h","window":{"kind":"five_hour"},"used_bp":5000,"resets_at":1794898800,"status":"allowed","severity":"ok","phase":"current"}"#
    );
    assert_eq!(
        json(&balance_meter(None)),
        r#"{"kind":"balance","id":"b:cny","currency":"cny","total_micros":110000000,"granted_micros":null,"topped_up_micros":null,"usable":true,"severity":"ok"}"#
    );
    assert_eq!(json(&UsageView::Pending), r#"{"state":"pending"}"#);
    assert_eq!(json(&UsageView::Unsupported), r#"{"state":"unsupported"}"#);
    assert_eq!(
        json(&UsageView::Unavailable {
            reason: Unavailable::Timeout
        }),
        r#"{"state":"unavailable","reason":"timeout"}"#
    );
    assert_eq!(
        json(&UsageView::NotConfigured {
            setup: Setup::UseGateway
        }),
        r#"{"state":"not_configured","setup":"use_gateway"}"#
    );
    let known = json(&UsageView::Known {
        reading: reading(Some(label("free")), true),
    });
    assert!(known.starts_with(r#"{"state":"known","reading":{"observed_at":1794890000000,"plan":"free","meters":[{"kind":"window","#), "{known}");
    let stale = json(&UsageView::Stale {
        reading: reading(None, false),
        since: 7,
        cause: StaleCause::Quiet,
    });
    assert!(
        stale
            .starts_with(r#"{"state":"stale","reading":{"observed_at":1794890000000,"plan":null,"#),
        "{stale}"
    );
    assert!(
        stale.ends_with(r#""since":7,"cause":{"kind":"quiet"}}"#),
        "{stale}"
    );

    let targets = [
        (RefreshTarget::All, r#"{"kind":"all"}"#),
        (
            RefreshTarget::Provider {
                provider: ProviderKey::Codex,
            },
            r#"{"kind":"provider","provider":"codex"}"#,
        ),
        (
            RefreshTarget::LiveCheck {
                provider: ProviderKey::Claude,
            },
            r#"{"kind":"live_check","provider":"claude"}"#,
        ),
    ];
    for (target, text) in targets {
        assert_eq!(json(&target), text);
        assert_eq!(serde_json::from_str::<RefreshTarget>(text).unwrap(), target);
    }

    assert_eq!(json(&Route::Overview), r#"{"name":"overview"}"#);
    assert_eq!(json(&Route::Settings), r#"{"name":"settings"}"#);
    assert_eq!(
        json(&Route::Provider {
            provider: ProviderKey::DeepSeek
        }),
        r#"{"name":"provider","provider":"deepseek"}"#
    );

    for (error, code) in [
        (CommandError::InvalidArgument, "invalid_argument"),
        (CommandError::Busy, "busy"),
        (CommandError::NotEnabled, "not_enabled"),
        (CommandError::StoreUnavailable, "store_unavailable"),
        (CommandError::Io, "io"),
        (CommandError::Internal, "internal"),
    ] {
        assert_eq!(json(&error), format!(r#"{{"code":"{code}"}}"#));
        let outcome: Outcome<Settings> = Outcome::Error { error };
        assert_eq!(
            json(&outcome),
            format!(r#"{{"status":"error","error":{{"code":"{code}"}}}}"#)
        );
    }
    assert_eq!(
        json(&Outcome::Ok { value: () }),
        r#"{"status":"ok","value":null}"#
    );
    assert_eq!(
        json(&Outcome::Ok {
            value: Route::Overview
        }),
        r#"{"status":"ok","value":{"name":"overview"}}"#
    );
    assert_eq!(
        Outcome::from(Ok::<u8, CommandError>(3)),
        Outcome::Ok { value: 3 }
    );
    assert_eq!(
        Outcome::<u8>::from(Err(CommandError::Busy)),
        Outcome::Error {
            error: CommandError::Busy
        }
    );
}

#[test]
fn structs_serialise_field_for_field() {
    assert_eq!(
        json(&refresh(false)),
        r#"{"in_flight":false,"last_attempt_at":null,"next_at":null,"free_refresh":true,"live_check":false,"live_check_error":null}"#
    );
    assert_eq!(
        json(&headline_without_reset()),
        r#"{"provider":"codex","meter_id":"w:30d","used_bp":400,"severity":"ok","resets_at":null}"#
    );
    assert_eq!(
        json(&series()),
        r#"{"provider":"claude","meter_id":"w:7d","from":1794800000,"to":1794886400,"unit":"basis_points","points":[{"t":1794800000,"v":3700,"lo":3600,"hi":3700}],"gap_s":4500,"resets":[1794850000]}"#
    );
    assert_eq!(
        json(&diagnostics(false)),
        r#"{"app_version":"0.1.0","sources":[{"source":"deepseek_balance","provider":"deepseek","last_ok_at":null,"last_error":null,"failures":2}],"history_bytes":4096,"history_samples":12,"tray_available":true}"#
    );
    let empty = Snapshot {
        generated_at: 5,
        providers: vec![],
        headline: None,
    };
    assert_eq!(
        json(&empty),
        r#"{"generated_at":5,"providers":[],"headline":null}"#
    );
    let provider = &snapshot(false).providers[3];
    assert_eq!(
        json(provider),
        r#"{"provider":"gemini","enabled":true,"usage":{"state":"unsupported"},"refresh":{"in_flight":false,"last_attempt_at":null,"next_at":null,"free_refresh":true,"live_check":false,"live_check_error":null}}"#
    );
}

#[test]
fn argument_types_validate_on_the_way_in() {
    let query: HistoryQuery =
        serde_json::from_str(r#"{"provider":"claude","meter_id":"w:n:7d_oi","range":"week"}"#)
            .unwrap();
    assert_eq!(query.provider, ProviderKey::Claude);
    assert_eq!(
        query.meter_id,
        MeterId::Window(WindowKind::Named {
            label: label("7d_oi")
        })
    );
    assert_eq!(query.range, HistoryRange::Week);
    assert_eq!(
        json(&query),
        r#"{"provider":"claude","meter_id":"w:n:7d_oi","range":"week"}"#
    );
    for bad in [
        r#"{"provider":"openai","meter_id":"w:5h","range":"day"}"#,
        r#"{"provider":"claude","meter_id":"w:n:NOT A KEY","range":"day"}"#,
        r#"{"provider":"claude","meter_id":"w:5h","range":"year"}"#,
        r#"{"provider":"claude","range":"day"}"#,
    ] {
        assert!(serde_json::from_str::<HistoryQuery>(bad).is_err(), "{bad}");
    }
    for bad in [
        r#"{"kind":"provider"}"#,
        r#"{"kind":"provider","provider":"x"}"#,
        r#"{"kind":"quit"}"#,
        r#""all""#,
    ] {
        assert!(serde_json::from_str::<RefreshTarget>(bad).is_err(), "{bad}");
    }
    assert!(serde_json::from_str::<SettingsPatch>(r#"{"zoom_pct":125}"#).is_ok());
}

/// Every object key under `value`, and every string leaf.
fn walk(value: &Value, keys: &mut BTreeSet<String>, strings: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                keys.insert(key.clone());
                walk(inner, keys, strings);
            }
        }
        Value::Array(items) => items.iter().for_each(|inner| walk(inner, keys, strings)),
        Value::String(s) => {
            strings.insert(s.clone());
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn allowlist() -> BTreeSet<String> {
    std::fs::read_to_string(fixtures().join("ipc-allowlist.txt"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn sample_values() -> Vec<Value> {
    let mut values = Vec::new();
    for some in [true, false] {
        values.push(serde_json::to_value(snapshot(some)).unwrap());
        values.push(serde_json::to_value(diagnostics(some)).unwrap());
        values.push(
            serde_json::to_value(Outcome::Ok {
                value: snapshot(some),
            })
            .unwrap(),
        );
    }
    values.push(serde_json::to_value(series()).unwrap());
    values.push(serde_json::to_value(Settings::defaults()).unwrap());
    values.push(serde_json::to_value(headline_without_reset()).unwrap());
    values.push(
        serde_json::to_value(Outcome::<Settings>::Error {
            error: CommandError::Busy,
        })
        .unwrap(),
    );
    values.push(
        serde_json::to_value(Route::Provider {
            provider: ProviderKey::Grok,
        })
        .unwrap(),
    );
    values.push(
        serde_json::to_value(RefreshTarget::LiveCheck {
            provider: ProviderKey::Claude,
        })
        .unwrap(),
    );
    values.push(serde_json::to_value(WindowKind::Rolling { minutes: 90 }).unwrap());
    values.push(
        serde_json::to_value(WindowKind::Named {
            label: label("7d_oi"),
        })
        .unwrap(),
    );
    values.push(
        serde_json::to_value(HistoryQuery {
            provider: ProviderKey::Codex,
            meter_id: MeterId::Balance(Currency::Other),
            range: HistoryRange::Quarter,
        })
        .unwrap(),
    );
    values
}

#[test]
fn every_key_is_on_the_allow_list_and_every_listed_key_is_used() {
    let allowed = allowlist();
    let (mut keys, mut strings) = (BTreeSet::new(), BTreeSet::new());
    for value in sample_values() {
        walk(&value, &mut keys, &mut strings);
    }
    let unknown: Vec<&String> = keys.difference(&allowed).collect();
    assert!(
        unknown.is_empty(),
        "keys missing from ipc-allowlist.txt: {unknown:?}"
    );
    let unused: Vec<&String> = allowed.difference(&keys).collect();
    assert!(
        unused.is_empty(),
        "allow-listed keys no IPC type has: {unused:?}"
    );
    // String leaves: closed vocabulary, labels, meter keys and the version (DESIGN 8.10).
    for s in &strings {
        let ok = !s.is_empty()
            && s.len() <= 48
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_:.+-".contains(&b));
        assert!(ok, "string leaf outside the closed shape: {s:?}");
    }
}

#[test]
fn allow_list_has_no_identity_or_secret_shaped_key() {
    let banned = [
        "email", "token", "secret", "password", "prompt", "message", "content", "account",
        "session", "cwd", "path", "url", "auth",
    ];
    for key in allowlist() {
        let lower = key.to_lowercase();
        assert!(!banned.iter().any(|b| lower.contains(b)), "{key}");
        assert!(!lower.ends_with("key"), "{key}");
        assert!(
            key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{key}"
        );
    }
}

/// The generated TypeScript: `cargo test -p jev-usage-core --features bindings export_bindings`.
#[cfg(feature = "bindings")]
mod bindings {
    use super::*;
    use ts_rs::TS;

    fn bindings_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/lib/ipc/bindings")
    }

    /// The file without its `//` line and `/** */` doc comments (the Rust doc text, which is prose).
    fn strip_comments(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find("/*") {
            out.push_str(&rest[..at]);
            rest = rest[at..].split_once("*/").map_or("", |(_, after)| after);
        }
        out.push_str(rest);
        out.lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<&str>>()
            .join("\n")
    }

    /// Writes one `.ts` file per IPC type (dependencies included) and returns the files written.
    fn export_all() -> Vec<(String, String)> {
        let dir = bindings_dir();
        Snapshot::export_all_to(&dir).unwrap();
        Outcome::<()>::export_all_to(&dir).unwrap();
        RefreshTarget::export_all_to(&dir).unwrap();
        HistoryQuery::export_all_to(&dir).unwrap();
        HistorySeries::export_all_to(&dir).unwrap();
        Settings::export_all_to(&dir).unwrap();
        SettingsPatch::export_all_to(&dir).unwrap();
        Diagnostics::export_all_to(&dir).unwrap();
        Route::export_all_to(&dir).unwrap();
        let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "ts"))
            .map(|path| {
                let text = std::fs::read_to_string(&path).unwrap();
                (
                    path.file_name().unwrap().to_str().unwrap().to_owned(),
                    strip_comments(&text),
                )
            })
            .collect();
        files.sort();
        files
    }

    const EXPECTED_FILES: [&str; 32] = [
        "CommandError.ts",
        "Currency.ts",
        "Diagnostics.ts",
        "Headline.ts",
        "HistoryPoint.ts",
        "HistoryQuery.ts",
        "HistoryRange.ts",
        "HistorySeries.ts",
        "HourCycle.ts",
        "LimitStatus.ts",
        "MeterView.ts",
        "Outcome.ts",
        "Phase.ts",
        "ProviderKey.ts",
        "ProviderView.ts",
        "ReadingView.ts",
        "RefreshTarget.ts",
        "RefreshView.ts",
        "Route.ts",
        "SeriesUnit.ts",
        "Settings.ts",
        "SettingsPatch.ts",
        "Setup.ts",
        "Severity.ts",
        "Snapshot.ts",
        "SourceDiag.ts",
        "SourceId.ts",
        "StaleCause.ts",
        "Theme.ts",
        "Unavailable.ts",
        "UsageView.ts",
        "WindowKind.ts",
    ];

    /// The declared type of `key` in a generated file: the text after `key: ` up to the
    /// next `,` or `}` at the same nesting depth.
    fn declared_types<'a>(file: &'a str, key: &str) -> Vec<&'a str> {
        let needle = format!("{key}: ");
        let mut found = Vec::new();
        let mut rest = file;
        while let Some(at) = rest.find(&needle) {
            let before = rest[..at].chars().next_back();
            let after = &rest[at + needle.len()..];
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                rest = after;
                continue;
            }
            let mut depth = 0_i32;
            let mut end = after.len();
            for (i, c) in after.char_indices() {
                match c {
                    '{' | '<' | '[' | '(' => depth += 1,
                    '}' | '>' | ']' | ')' if depth > 0 => depth -= 1,
                    ',' | '}' | ';' if depth == 0 => {
                        end = i;
                        break;
                    }
                    _ => {}
                }
            }
            found.push(after[..end].trim());
            rest = &after[end..];
        }
        found
    }

    /// Every key that is `null` somewhere under `value`.
    fn null_keys(value: &Value, out: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                for (key, inner) in map {
                    if inner.is_null() {
                        out.insert(key.clone());
                    }
                    null_keys(inner, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|inner| null_keys(inner, out)),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    #[test]
    fn export_bindings() {
        let files = export_all();
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, EXPECTED_FILES, "the set of generated files changed");

        // 1a: generated code is under the same type rules as hand-written code.
        for (name, text) in &files {
            for banned in [
                "any",
                "unknown",
                "never",
                "bigint",
                "object",
                "Function",
                "undefined",
            ] {
                // String literals ("unknown" is a severity) are values, not types.
                let code: String = text.split('"').step_by(2).collect::<Vec<&str>>().join(" ");
                let hit = code
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .any(|word| word == banned);
                assert!(!hit, "{name} contains the type `{banned}`: {text}");
            }
            assert!(!text.contains(" as "), "{name} contains an assertion");
        }

        // Every key that comes out `null` at run time is typed `| null` (DESIGN 7.4).
        let mut nulls = BTreeSet::new();
        null_keys(&serde_json::to_value(snapshot(false)).unwrap(), &mut nulls);
        null_keys(
            &serde_json::to_value(diagnostics(false)).unwrap(),
            &mut nulls,
        );
        null_keys(
            &serde_json::to_value(headline_without_reset()).unwrap(),
            &mut nulls,
        );
        let expected: BTreeSet<String> = [
            "granted_micros",
            "headline",
            "last_attempt_at",
            "last_error",
            "last_ok_at",
            "live_check_error",
            "next_at",
            "plan",
            "resets_at",
            "topped_up_micros",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(nulls, expected, "the set of nullable keys changed");
        for key in &nulls {
            let mut seen = 0;
            for (name, text) in &files {
                for declared in declared_types(text, key) {
                    seen += 1;
                    assert!(
                        declared.ends_with("| null"),
                        "{name}: `{key}` can be null but is typed `{declared}`"
                    );
                }
            }
            assert!(seen > 0, "`{key}` not found in any generated file");
        }
        // And the converse for the number-typed overrides: no bare `number` on an optional field.
        let optional_number_keys = [
            "resets_at",
            "granted_micros",
            "topped_up_micros",
            "last_attempt_at",
            "next_at",
            "last_ok_at",
        ];
        for key in optional_number_keys {
            for (_, text) in &files {
                for declared in declared_types(text, key) {
                    assert_eq!(declared, "number | null", "{key}");
                }
            }
        }

        // `SettingsPatch`: every key optional, none nullable-only, and no `version`.
        let patch = &files
            .iter()
            .find(|(name, _)| name == "SettingsPatch.ts")
            .unwrap()
            .1;
        let settings = &files
            .iter()
            .find(|(name, _)| name == "Settings.ts")
            .unwrap()
            .1;
        let settings_json = serde_json::to_value(Settings::defaults()).unwrap();
        for key in settings_json.as_object().unwrap().keys() {
            assert!(
                settings.contains(&format!("{key}: ")),
                "Settings.ts lacks {key}"
            );
            if key == "version" {
                assert!(!patch.contains("version"), "SettingsPatch has a version");
            } else {
                assert!(
                    patch.contains(&format!("{key}?: ")),
                    "SettingsPatch.{key} is not optional: {patch}"
                );
            }
        }

        // The property names of the generated files are on the allow-list too (DESIGN 8.10).
        let allowed = allowlist();
        for (name, text) in &files {
            let body = text.split_once("export type").unwrap().1;
            let mut rest = body;
            while let Some(at) = rest.find(": ") {
                let key: String = rest[..at]
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect::<Vec<char>>()
                    .into_iter()
                    .rev()
                    .collect();
                let quoted = rest[..at].ends_with('"');
                if !key.is_empty() && !quoted {
                    assert!(
                        allowed.contains(&key),
                        "{name}: property `{key}` is not on the allow-list"
                    );
                }
                rest = &rest[at + 2..];
            }
        }

        // The generic envelope really is generic.
        let outcome = &files
            .iter()
            .find(|(name, _)| name == "Outcome.ts")
            .unwrap()
            .1;
        assert!(outcome.contains("export type Outcome<T> ="), "{outcome}");
    }
}
