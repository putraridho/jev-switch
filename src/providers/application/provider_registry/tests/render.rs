//! The `/providers` text.

use super::*;
use crate::providers::domain::availability;

    #[test]
    fn render_mentions_cooldown_refusals_allowances_and_overrides() {
        let mut rt = runtime();
        rt.cooldowns.start(Claude, None, NOW);
        rt.mark_unusable(Codex, "some-model");
        rt.overrides.notes.push("JEV_CHAINS ignored: boom".to_string());
        let text = rt.render(NOW + 12 * MINUTE_MS);
        assert!(text.contains("claude cooling for 48m"), "{text}");
        assert!(!text.contains("codex cooling"), "{text}");
        assert!(text.contains("codex:some-model refused"), "{text}");
        assert!(text.contains("credits no, per-token no"), "{text}");
        assert!(text.contains("override: JEV_CHAINS ignored: boom"), "{text}");
        assert!(text.contains(&availability::render::render(&rt.avail)), "{text}");
    }

    #[test]
    fn render_without_events_has_no_cooldown_lines() {
        assert!(!runtime().render(NOW).contains("cooling"));
    }
