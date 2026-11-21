//! Session stickiness for the code-mode tiers.

use crate::judgment::domain::thresholds::MAX_HOLD;
use crate::judgment::domain::want::CodeWant;

/// Session stickiness for code-mode tiers. Prompt caches are per model, so a
/// switch rebuilds the whole context cache: upgrade at once, but step down (one
/// tier, never below mid) only after `downgrade_after` confident easier turns.
#[derive(Debug, Default, Clone)]
pub struct Sticky {
    /// Tier of the model that answered the previous turn.
    pub tier: Option<usize>,
    lower_streak: u32,
    /// Consecutive turns wanting a lower tier than `tier`, confident or not.
    held: u32,
}

impl Sticky {
    /// The tier for this turn given Jev's `want`. Callers set `tier` once the
    /// turn runs on it.
    pub fn decide(&mut self, w: &CodeWant, downgrade_after: u32) -> (usize, String) {
        let Some(cur) = self.tier else {
            return (w.tier, w.reason.clone());
        };
        if w.tier > cur {
            self.lower_streak = 0;
            self.held = 0;
            return (w.tier, format!("upgrade: {}", w.reason));
        }
        if w.tier == cur {
            self.lower_streak = 0;
            self.held = 0;
            return (cur, format!("stay (warm cache): {}", w.reason));
        }
        self.held += 1;
        if self.held >= MAX_HOLD && cur > 1 {
            self.lower_streak = 0;
            self.held = 0;
            return (cur - 1, format!("downgrade after holding {MAX_HOLD} easier turns: {}", w.reason));
        }
        if !w.confident {
            // Uncertain turn: keep the streak so one vague message doesn't erase progress.
            return (cur, format!("stay (warm cache): {}", w.reason));
        }
        self.lower_streak += 1;
        if self.lower_streak >= downgrade_after {
            self.lower_streak = 0;
            self.held = 0;
            return ((cur - 1).max(1), format!("downgrade after {downgrade_after} easier turns: {}", w.reason));
        }
        (cur, format!("stay ({}/{} toward downgrade): {}", self.lower_streak, downgrade_after, w.reason))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn want(tier: usize, confident: bool) -> CodeWant {
        CodeWant { tier, confident, reason: String::new(), signals: Value::Null }
    }

    #[test]
    fn uncertain_turn_holds_downgrade_streak() {
        let mut s = Sticky { tier: Some(2), ..Default::default() };
        assert_eq!(s.decide(&want(1, true), 2).0, 2); // 1/2
        assert_eq!(s.decide(&want(1, false), 2).0, 2); // vague turn: streak kept
        assert_eq!(s.decide(&want(1, true), 2).0, 1); // 2/2: step down
    }

    #[test]
    fn top_tier_steps_down_one_tier_at_a_time() {
        let mut s = Sticky { tier: Some(2), ..Default::default() };
        let (t, why) = s.decide(&want(3, true), 2);
        assert_eq!((t, why.starts_with("upgrade")), (3, true));
        s.tier = Some(3);
        assert_eq!(s.decide(&want(1, true), 2).0, 3);
        let (t, _) = s.decide(&want(1, true), 2);
        assert_eq!(t, 2, "Fable steps down to Opus, not straight to Sonnet");
        s.tier = Some(2);
        assert_eq!(s.decide(&want(1, true), 2).0, 2);
    }

    #[test]
    fn hard_turn_resets_downgrade_streak() {
        let mut s = Sticky { tier: Some(2), ..Default::default() };
        assert_eq!(s.decide(&want(1, true), 2).0, 2);
        assert_eq!(s.decide(&want(2, true), 2).0, 2); // needs Opus: streak reset
        assert_eq!(s.decide(&want(1, true), 2).0, 2); // back to 1/2
    }

    #[test]
    fn vague_turns_cannot_hold_a_tier_forever() {
        let mut s = Sticky { tier: Some(2), ..Default::default() };
        for _ in 1..MAX_HOLD {
            assert_eq!(s.decide(&want(1, false), 2).0, 2);
        }
        assert_eq!(s.decide(&want(1, false), 2).0, 1);
    }
}
