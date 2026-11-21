//! What a failed attempt does to the runtime: blocks, refusals and their clearing.

use std::collections::HashSet;

use super::*;
use crate::backend::{ErrorKind, TurnError};
use crate::providers::application::provider_registry::BlockReason;

    #[test]
    fn a_retired_cli_blocks_the_provider_for_the_whole_session() {
        let ready = |p| Availability { provider: p, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true };
        let mut rt = Runtime::for_tests(vec![ready(ProviderId::Claude), ready(ProviderId::Gemini)]);
        let mut excluded = HashSet::new();
        let err = TurnError { kind: ErrorKind::Unsupported, message: "This client is no longer supported".into() };
        let note = rt.react(ProviderId::Gemini, "gemini-3.8-flash", &err, &mut excluded, 1_000).unwrap();
        assert!(note.contains("Antigravity") && note.contains("trying the next model"), "{note}");
        assert_eq!(rt.blocked.get(&ProviderId::Gemini), Some(&BlockReason::Unsupported));
        rt.forget_unusable_here();
        assert_eq!(rt.blocked.get(&ProviderId::Gemini), Some(&BlockReason::Unsupported), "/new does not clear it");
        assert!(rt.render(1_000).contains("gemini blocked this session"));
    }

    #[test]
    fn a_directory_refusal_blocks_the_provider_with_a_reason_until_new_session() {
        let ready = |p| Availability { provider: p, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true };
        let mut rt = Runtime::for_tests(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        let mut excluded = HashSet::new();
        let err = TurnError { kind: ErrorKind::UnusableHere, message: "Not inside a trusted directory".into() };
        let note = rt.react(ProviderId::Codex, "gpt-6-luna", &err, &mut excluded, 1_000).unwrap();
        assert!(note.contains("codex needs a git repository here") && note.contains("trying the next model"), "{note}");
        assert_eq!(rt.blocked.get(&ProviderId::Codex), Some(&BlockReason::UnusableHere));
        assert!(rt.cooldowns.active(ProviderId::Codex, 1_000).is_none(), "not a usage limit");
        assert!(!rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Claude, "claude-sonnet-5-5").unwrap()));
        assert!(rt.render(1_000).contains("codex blocked this session: needs a git repository here"), "{}", rt.render(1_000));
        // A probe that sees codex logged in does not change the directory ...
        rt.apply_probe(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        assert_eq!(rt.blocked.get(&ProviderId::Codex), Some(&BlockReason::UnusableHere));
        // ... but a login block still clears while this one stays; /new clears this one.
        rt.blocked.insert(ProviderId::Claude, BlockReason::Auth);
        rt.apply_probe(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        assert_eq!(rt.blocked.len(), 1);
        rt.forget_unusable_here();
        assert!(rt.blocked.is_empty());
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
    }

    #[test]
    fn a_failed_login_is_session_only_and_clears_when_the_probe_sees_a_login() {
        let ready = |p| Availability { provider: p, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true };
        let mut rt = Runtime::for_tests(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        let mut excluded = HashSet::new();
        let err = TurnError { kind: ErrorKind::Auth, message: "401".into() };
        let note = rt.react(ProviderId::Codex, "gpt-6-luna", &err, &mut excluded, 1_000).unwrap();
        assert!(note.contains("log in"), "{note}");
        // not in the persisted cooldowns, but not offered either
        assert!(rt.cooldowns.active(ProviderId::Codex, 1_000).is_none());
        assert!(!rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Claude, "claude-sonnet-5-5").unwrap()));
        // a probe that still says "not logged in" keeps it blocked ...
        let mut not_logged = ready(ProviderId::Codex);
        not_logged.state = State::NotLoggedIn;
        rt.apply_probe(vec![ready(ProviderId::Claude), not_logged]);
        assert_eq!(rt.blocked.get(&ProviderId::Codex), Some(&BlockReason::Auth));
        // ... and one that sees the login again offers it right away
        rt.apply_probe(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        assert!(rt.blocked.is_empty());
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
    }
