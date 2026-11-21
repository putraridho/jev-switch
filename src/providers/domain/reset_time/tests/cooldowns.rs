use super::super::cooldowns::Cooldowns;
use super::super::{DEFAULT_MS, MINUTE_MS};
use super::{never, NOW};
use crate::shared::ProviderId::{Claude, Codex};

#[test]
fn active_counts_down_then_expires() {
    let mut cooldowns = Cooldowns::default();
    assert_eq!(cooldowns.active(Claude, NOW), None);
    cooldowns.start_in(Claude, None, NOW, &never, &|_| false);
    assert_eq!(cooldowns.active(Claude, NOW), Some(DEFAULT_MS));
    assert_eq!(cooldowns.active(Claude, NOW + DEFAULT_MS - 1), Some(1));
    assert_eq!(cooldowns.active(Claude, NOW + DEFAULT_MS), None);
    assert_eq!(cooldowns.active(Codex, NOW), None);
}

#[test]
fn start_with_uses_the_resolver() {
    let mut cooldowns = Cooldowns::default();
    cooldowns.start_in(Codex, Some("20m"), NOW, &never, &|_| false);
    assert_eq!(cooldowns.active(Codex, NOW), Some(20 * MINUTE_MS));
}

#[test]
fn clear_removes() {
    let mut cooldowns = Cooldowns::default();
    cooldowns.start_in(Claude, None, NOW, &never, &|_| false);
    cooldowns.clear(Claude);
    assert_eq!(cooldowns.active(Claude, NOW), None);
}
