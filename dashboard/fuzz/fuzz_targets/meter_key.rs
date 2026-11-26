#![no_main]

use libfuzzer_sys::fuzz_target;

use jev_usage_core::meter::MeterId;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Some(id) = MeterId::parse_key(text) {
            // `parse_key` is the exact inverse of `key`.
            assert_eq!(id.key(), text);
        }
    }
});
