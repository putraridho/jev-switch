#![no_main]

use libfuzzer_sys::fuzz_target;

use jev_usage_core::parse::deepseek;

fuzz_target!(|data: &[u8]| {
    let _ = deepseek::parse_balance(data);
});
