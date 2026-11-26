#![no_main]

use libfuzzer_sys::fuzz_target;

use jev_usage_core::quantity::Money;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = Money::parse_decimal(text);
    }
});
