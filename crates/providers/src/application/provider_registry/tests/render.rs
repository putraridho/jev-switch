//! The `/providers` text.

use super::*;
use crate::domain::availability;

    #[test]
    fn render_mentions_cooldown_refusals_allowances_and_overrides() {
        let mut rt = runtime();
        rt.cooldowns.start_in(Claude, None, NOW, &|_, _| None, &|_| false);
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

    #[test]
    fn render_says_how_a_ready_provider_is_funded_and_never_claims_a_plan_it_cannot_see() {
        use crate::domain::funding::{Funding, Fundings};
        let mut rt = runtime();
        assert!(!rt.render(NOW).contains("billing:"), "nothing inspected, nothing claimed");
        rt.fundings = Fundings::default().with(Claude, Funding::Login).with(Codex, Funding::Custom).with(Gemini, Funding::ApiKey);
        let text = rt.render(NOW);
        assert!(text.contains("billing: claude Claude login\n"), "{text}");
        assert!(text.contains("billing: codex custom endpoint (billing unknown); counts against the paid cap"), "{text}");
        assert!(!text.contains("billing: gemini"), "not ready: {text}");
    }
