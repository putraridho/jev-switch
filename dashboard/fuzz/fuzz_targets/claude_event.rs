#![no_main]

use libfuzzer_sys::fuzz_target;

use jev_usage_core::parse::claude_event;

fuzz_target!(|data: &[u8]| {
    let _ = claude_event::parse_line(data);
});
