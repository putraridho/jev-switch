//! Tests for reset-time parsing and the cooldown store, by subject. Shared helpers live here.

mod cooldowns;
mod duration;
mod instant;
mod next_year;
mod wall_clock;
mod zone;

use std::cell::RefCell;
use std::path::PathBuf;

use super::{cooldown_ms_in, Resolver};

pub(super) const NOW: u64 = 1_760_000_000_000;

/// 2026-12-31 12:00:00 UTC.
pub(super) const NEW_YEARS_EVE: u64 = 1_798_718_400_000;

/// A resolver that must never be reached.
pub(super) fn never(_: &str, _: Option<&str>) -> Option<u64> {
    panic!("the resolver must not be called");
}

pub(super) fn unresolvable(_: &str, _: Option<&str>) -> Option<u64> {
    None
}

/// Only these zones "exist" in tests, so no test depends on the machine's zone database.
fn fake_zones(zone: &str) -> bool {
    matches!(zone, "Asia/Jakarta" | "America/Los_Angeles")
}

pub(super) fn cooldown_in(text: Option<&str>, now_ms: u64, resolve: Resolver) -> u64 {
    cooldown_ms_in(text, now_ms, resolve, &fake_zones)
}

/// A resolver that returns a fixed instant and records what it was asked.
pub(super) fn fixed(at_secs: u64, seen: &RefCell<Vec<(String, Option<String>)>>) -> impl Fn(&str, Option<&str>) -> Option<u64> + '_ {
    move |text, zone| {
        seen.borrow_mut().push((text.to_string(), zone.map(str::to_string)));
        Some(at_secs)
    }
}

pub(super) fn temp_dir(name: &str) -> PathBuf {
    crate::testkit::scratch_dir::scratch_dir("cooldown", name)
}
