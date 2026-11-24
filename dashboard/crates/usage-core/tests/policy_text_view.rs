//! Formatting and read-model policy tests.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)] // Test fixtures use checked assertions and synthetic values.
use jev_usage_core::{
    ipc::{HourCycle, RefreshView},
    label::Label,
    meter::WindowKind,
    provider::ProviderKey,
    quantity::Percent,
    settings::Settings,
    text::{self, LocalStamp, LocalTime},
    time::{UnixMillis, UnixSeconds},
    usage::Usage,
    view::{self, ProviderState},
};
use serde_json::Value;

struct Jakarta;
impl LocalTime for Jakarta {
    fn local(&self, at: UnixSeconds) -> LocalStamp {
        let seconds = i64::try_from(at.0).unwrap() + 7 * 3600;
        let days = seconds.div_euclid(86400);
        let rem = seconds.rem_euclid(86400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let mut year = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = mp + if mp < 10 { 3 } else { -9 };
        year += i64::from(month <= 2);
        LocalStamp {
            year: i32::try_from(year).unwrap(),
            month: u8::try_from(month).unwrap(),
            day: u8::try_from(day).unwrap(),
            weekday: u8::try_from((days + 3).rem_euclid(7)).unwrap(),
            hour: u8::try_from(rem / 3600).unwrap(),
            minute: u8::try_from(rem % 3600 / 60).unwrap(),
        }
    }
}
#[test]
fn shared_format_vectors() {
    let vectors: Value =
        serde_json::from_str(include_str!("../../../fixtures/format-vectors.json")).unwrap();
    for v in vectors["percent"].as_array().unwrap() {
        assert_eq!(
            text::percent(Percent::from_basis_points(u32::try_from(v["bp"].as_u64().unwrap()).unwrap())),
            v["expect"].as_str().unwrap(),
            "{}",
            v["id"]
        );
    }
    for v in vectors["countdown"].as_array().unwrap() {
        let actual = text::countdown(
            UnixSeconds(v["resets_at_s"].as_u64().unwrap()),
            UnixSeconds(v["now_ms"].as_u64().unwrap() / 1000),
        );
        assert_eq!(actual.as_deref(), v["expect"].as_str(), "{}", v["id"]);
    }
    for v in vectors["absolute"].as_array().unwrap() {
        let hour = if v["hour"] == "h12" {
            HourCycle::H12
        } else {
            HourCycle::H24
        };
        assert_eq!(
            text::reset_absolute(
                UnixSeconds(v["resets_at_s"].as_u64().unwrap()),
                UnixSeconds(v["now_ms"].as_u64().unwrap() / 1000),
                &Jakarta,
                hour
            ),
            v["expect"].as_str().unwrap(),
            "{}",
            v["id"]
        );
    }
    for v in vectors["window_name"].as_array().unwrap() {
        let w = &v["window"];
        let kind = match w["kind"].as_str().unwrap() {
            "five_hour" => WindowKind::FiveHour,
            "seven_day" => WindowKind::SevenDay,
            "monthly" => WindowKind::Monthly,
            "rolling" => WindowKind::Rolling {
                minutes: u32::try_from(w["minutes"].as_u64().unwrap()).unwrap(),
            },
            "named" => WindowKind::Named {
                label: Label::parse(w["label"].as_str().unwrap()).unwrap(),
            },
            _ => panic!("unexpected fixture"),
        };
        assert_eq!(
            text::window_name(&kind),
            v["expect"].as_str().unwrap(),
            "{}",
            v["id"]
        );
    }
}
#[test]
fn snapshot_orders_and_ignores_generation_time() {
    let refresh = RefreshView {
        in_flight: false,
        last_attempt_at: None,
        next_at: None,
        free_refresh: true,
        live_check: false,
        live_check_error: None,
    };
    let pending = Usage::Pending;
    let states = [
        ProviderState {
            provider: ProviderKey::Codex,
            usage: &pending,
            refresh: refresh.clone(),
        },
        ProviderState {
            provider: ProviderKey::Claude,
            usage: &pending,
            refresh,
        },
    ];
    let settings = Settings::defaults();
    let first = view::snapshot(&states, UnixMillis(1), &settings);
    let second = view::snapshot(&states, UnixMillis(2), &settings);
    assert!(view::same_content(&first, &second));
    assert_eq!(first.providers[0].provider, ProviderKey::Claude);
    assert_eq!(
        text::tray_rows(&first, UnixSeconds(0), &Jakarta, HourCycle::H24)[0].text,
        "Claude · checking"
    );
}
