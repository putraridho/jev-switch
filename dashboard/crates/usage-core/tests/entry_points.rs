//! The stable byte-level mirrors of the fuzz targets for the three D11.2 entry points that
//! `parse_security.rs` does not cover: `Money::parse_decimal`, `MeterId::parse_key` and the
//! settings JSON. The same bodies as `dashboard/fuzz/fuzz_targets/*.rs`.
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

use jev_usage_core::meter::MeterId;
use jev_usage_core::quantity::Money;
use jev_usage_core::settings::{Settings, SettingsPatch};
use proptest::prelude::*;

proptest! {
    #[test]
    fn money_parse_decimal_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            let _ = Money::parse_decimal(text);
        }
    }

    #[test]
    fn money_parse_decimal_never_panics_on_decimal_shapes(text in "[-+0-9.,eE ]{0,40}") {
        let _ = Money::parse_decimal(&text);
    }

    #[test]
    fn meter_key_is_the_exact_inverse_of_key(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if let Some(id) = MeterId::parse_key(text) {
                prop_assert_eq!(id.key(), text);
            }
        }
    }

    #[test]
    fn meter_key_shapes_round_trip_or_reject(text in "(w|b):[a-z0-9_:]{0,16}") {
        if let Some(id) = MeterId::parse_key(&text) {
            prop_assert_eq!(id.key(), text);
        }
    }

    #[test]
    fn settings_json_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        if let Ok(settings) = serde_json::from_slice::<Settings>(&bytes) {
            let _ = settings.validated();
        }
        if let Ok(patch) = serde_json::from_slice::<SettingsPatch>(&bytes) {
            let _ = Settings::defaults().apply(patch);
        }
    }

    #[test]
    fn settings_patch_with_arbitrary_numbers_never_panics(
        warn in any::<u16>(), critical in any::<u16>(), zoom in any::<u16>()
    ) {
        let json = format!(r#"{{"warn_bp":{warn},"critical_bp":{critical},"zoom_pct":{zoom}}}"#);
        if let Ok(patch) = serde_json::from_slice::<SettingsPatch>(json.as_bytes()) {
            let _ = Settings::defaults().apply(patch);
        }
    }
}
