//! The calibrated cutoffs of routing and orchestration (docs/guide.md says how each was set).

/// `needs_changes` probability at or above which a code turn counts as editing.
pub const CODE_EDIT_P: f64 = 0.5;
/// Code mode: `security_review` probability at or above which a turn goes strong
/// even with no security keyword ("can a viewer see X?", "who can call this?").
/// On lunagraph (2,296 prompts, 408 Sonnet answers graded against the code), the
/// keyword-less security questions Sonnet missed and Opus fixed scored 0.86-0.97;
/// 0.85 catches all but one at 0.60, for 0.9% more turns on Opus than 0.90.
pub const CODE_SECURITY_P: f64 = 0.85;
/// Code mode default for `CODE_STRONG_THRESHOLD`: Jev `reasoning` confidence at or
/// above which a turn goes strong. On lunagraph, Sonnet's answers with reasoning
/// confidence 0.75-0.85 were 31 correct and 2 partial that Opus didn't fix either;
/// its misses Opus fixed scored 0.89 and 0.92. Raising it from 0.75 cut Opus turns
/// from 32% to 28% with no fixable miss lost.
pub const CODE_STRONG_THRESHOLD: f64 = 0.85;

/// Orchestrate mode default for `ORCH_TRIGGER_P`: `multi_part` probability at or above
/// which a prompt is orchestrated. Scored on 1,551 distinct logged prompts (2026-11-16,
/// `orch_tune`): 12 (0.8%) scored 0.70 or more, and all but one short follow-up were
/// real multi-part work (rebuild an app, 14 tickets, 20 page variants, four issues in
/// one PR). The 0.60-0.70 band starts mixing in follow-ups and single asks.
pub const ORCH_TRIGGER_P: f64 = 0.70;

/// `sequential` probability at or above which a multi-part request is NOT orchestrated:
/// steps that must run in order ("run the tests, then check the report, then refactor",
/// "do 1 -> 2 -> 3") gain nothing from parallel workers. A veto, not a rescaling, so
/// the calibrated `multi_part` scores of genuinely parallel work are untouched.
pub const SEQUENTIAL_VETO_P: f64 = 0.70;

/// Default of the gateway's `FABLE_THRESHOLD` environment variable (the one name users
/// set; this constant says "top" because it is the top tier, whatever model fills it):
/// `very_hard` probability at or above which a turn that would go to Opus goes to
/// Fable instead. `jev-code` routing uses the same value, not configurable. NOT
/// calibrated: there is no graded Opus-versus-Fable comparison yet, and Fable's price is
/// unknown. The cutoff is high so few turns qualify; the gateway logs `hard_p` on every Opus-bound turn so it can
/// be tuned from real usage. `JEV_FABLE=off` disables the tier.
pub const TOP_THRESHOLD: f64 = 0.90;

/// Most consecutive easier turns (confident or not) a tier is held for. A cache
/// rebuild costs about one turn's worth of re-reading the context at the new model's
/// write price; against a ~5x Opus premium that is repaid in ~3 turns, so holding
/// Opus through 4+ vague turns costs more than it saves. On the logged turns, 5 of
/// 24 holds ran 4-9 turns, all of them unclear/low-confidence ones that never counted
/// toward `downgrade_after`.
pub const MAX_HOLD: u32 = 4;

/// Default `JEV_TIMEOUT_MS`: the longest a routing judgment may take before the turn
/// goes ahead on the current model. A judgment normally takes ~0.5 s (three questions
/// in parallel); without a limit a stalled connection held the turn until it broke.
pub const JEV_TIMEOUT_MS: u64 = 3000;
