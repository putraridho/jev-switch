//! `fixture!("claude/turn1.jsonl")` embeds `tests/fixtures/claude/turn1.jsonl` as a `&'static str`.

/// The one place that knows where `tests/fixtures/` lives relative to the crate root.
/// When the crate moves two levels down, change the prefix below to `/../../tests/fixtures/`.
macro_rules! fixture {
    ($path:expr) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $path))
    };
}
