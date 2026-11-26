//! The Antigravity `/usage` allow-list parser (DESIGN 6.5).
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

use jev_usage_core::meter::{LimitStatus, Meter, Meters, WindowReading};
use jev_usage_core::parse::antigravity::parse_usage;
use jev_usage_core::text::window_name;
use jev_usage_core::time::UnixSeconds;
use jev_usage_core::usage::Unavailable;
use proptest::prelude::*;

/// 2026-11-26T04:40:00Z.
const NOW: UnixSeconds = UnixSeconds(1_795_668_000);

fn answer(buckets: &[(&str, &str, &str)]) -> Vec<u8> {
    let list: Vec<String> = buckets
        .iter()
        .map(|(id, remaining, reset)| {
            format!(
                r#"{{"id":"{id}","name":"JEVCANARY","description":"JEVCANARY","remaining_fraction":{remaining},"reset_time":"{reset}"}}"#
            )
        })
        .collect();
    format!(
        r#"{{"conversation_id":"JEVCANARY","status":"SUCCESS","response":"JEVCANARY","command":{{"data":{{"groups":[{{"name":"JEVCANARY","buckets":[{}]}}]}}}}}}"#,
        list.join(",")
    )
    .into_bytes()
}

fn windows(meters: &Meters) -> Vec<(String, u16, Option<u64>, LimitStatus)> {
    meters
        .as_slice()
        .iter()
        .map(|m| match m {
            Meter::Window(WindowReading {
                kind,
                used,
                resets_at,
                status,
            }) => (
                window_name(kind),
                used.basis_points(),
                resets_at.map(|r| r.0),
                *status,
            ),
            Meter::Balance(_) => panic!("no balance"),
        })
        .collect()
}

#[test]
fn the_four_known_buckets_become_named_windows_in_a_fixed_order() {
    let output = answer(&[
        ("3p-weekly", "1", "1970-01-01T00:00:00Z"),
        ("gemini-weekly", "0.75", "2026-12-01T05:44:11Z"),
        ("unknown-bucket", "0.1", "2026-12-01T05:44:11Z"),
        ("gemini-5h", "0", "2026-11-26T07:05:23.5Z"),
        ("3p-5h", "1", "1970-01-01T00:00:00Z"),
    ]);
    let meters = parse_usage(&output, NOW).unwrap();
    assert!(!format!("{meters:?}").contains("JEVCANARY"));
    assert_eq!(
        windows(&meters),
        vec![
            (
                "Gemini 5-hour".to_owned(),
                10_000,
                Some(1_795_676_723),
                LimitStatus::Rejected
            ),
            (
                "Gemini 7-day".to_owned(),
                2_500,
                Some(1_796_103_851),
                LimitStatus::Allowed
            ),
            (
                "Claude/GPT 5-hour".to_owned(),
                0,
                None,
                LimitStatus::Allowed
            ),
            ("Claude/GPT 7-day".to_owned(), 0, None, LimitStatus::Allowed),
        ]
    );
}

#[test]
fn the_blank_answer_of_a_signed_out_tool_is_never_real_data() {
    // Every window full; resets exactly one window ahead, or absent.
    let output = answer(&[
        ("gemini-weekly", "1", "2026-12-03T04:40:30Z"),
        ("gemini-5h", "1", "2026-11-26T09:40:00Z"),
        ("3p-weekly", "1", "1970-01-01T00:00:00Z"),
        ("3p-5h", "1", "1970-01-01T00:00:00Z"),
    ]);
    assert_eq!(parse_usage(&output, NOW), Err(Unavailable::NotLoggedIn));
}

#[test]
fn a_full_window_with_a_real_reset_is_data() {
    let output = answer(&[("gemini-5h", "1", "2026-11-26T07:05:23Z")]);
    assert!(parse_usage(&output, NOW).is_ok());
}

#[test]
fn other_shapes_are_errors() {
    assert_eq!(parse_usage(b"not json", NOW), Err(Unavailable::Protocol));
    assert_eq!(
        parse_usage(br#"{"status":"ERROR"}"#, NOW),
        Err(Unavailable::ServerError)
    );
    assert_eq!(
        parse_usage(br#"{"status":"SUCCESS"}"#, NOW),
        Err(Unavailable::Protocol)
    );
    let out_of_range = answer(&[("gemini-5h", "1.5", "2026-11-26T07:05:23Z")]);
    assert_eq!(parse_usage(&out_of_range, NOW), Err(Unavailable::Protocol));
}

#[test]
fn a_reset_that_is_not_a_utc_time_is_unknown() {
    for bad in [
        "",
        "tomorrow",
        "2026-13-01T00:00:00Z",
        "2026-11-26T07:05:23+07:00",
        "2026-11-26T24:00:00Z",
        "99999-01-01T00:00:00Z",
    ] {
        let meters = parse_usage(&answer(&[("gemini-5h", "0.5", bad)]), NOW).unwrap();
        assert_eq!(windows(&meters)[0].2, None, "{bad}");
    }
}

#[test]
fn a_distant_reset_is_dropped_by_the_plausibility_guard() {
    let output = answer(&[("gemini-5h", "0.5", "2100-01-01T00:00:00Z")]);
    let meters = parse_usage(&output, NOW)
        .unwrap()
        .without_implausible_resets(jev_usage_core::time::UnixMillis(NOW.0 * 1000));
    assert_eq!(windows(&meters)[0].2, None);
}

proptest! {
    #[test]
    fn never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512), now in any::<u64>()) {
        let _ = parse_usage(&bytes, UnixSeconds(now));
    }

    #[test]
    fn any_reset_text_is_safe(reset in "[0-9TZ:.+-]{0,30}") {
        let _ = parse_usage(&answer(&[("gemini-5h", "0.5", &reset)]), NOW);
    }
}
