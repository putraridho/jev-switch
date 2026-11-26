#![no_main]

use libfuzzer_sys::fuzz_target;

use jev_usage_core::parse::claude_log;

fuzz_target!(|data: &[u8]| {
    let sample = claude_log::parse_line(data);
    // The cheap pre-filter must never reject a line the parser accepts.
    assert!(sample.is_none() || claude_log::might_have_sample(data));
});
