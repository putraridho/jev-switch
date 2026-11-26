#![no_main]

use libfuzzer_sys::fuzz_target;

use jev_usage_core::parse::codex;

fuzz_target!(|data: &[u8]| {
    let _ = codex::parse_rate_limits_response(data, 2);
});
