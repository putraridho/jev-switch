//! Asking routing for the one model of an attempt, or the legacy tier when routing is off.

use std::collections::HashSet;

use crate::catalog::domain::model_table::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use crate::backend::{ProviderId};
use crate::catalog::domain::model_table as catalog;
use crate::catalog::domain::model_table::{find, ModelSpec};
use crate::turn::application::guards::TurnFacts;
use crate::judgment::domain::want::CodeWant;
use crate::routing::domain::decide as routing;
use crate::routing::domain::decide::{NoRoute, Pin, PinOrigin, RouteInput, RoutingState, Signals};
use crate::turn::application::judged::Judged;
use crate::turn::domain::route_facts::RouteFacts;
use super::attempt::Chosen;
use super::{Engine, CATCH_UP_TOKENS};

/// The Claude models of the legacy (routing off) tiers, cheapest first.
pub(super) const LEGACY_TIERS: [&str; 3] = [MODEL_CHEAP, MODEL_MID, MODEL_STRONG];

/// What one route decision is made from: the request and the state it is judged against.
pub(super) struct ChooseInput<'a> {
    pub(super) judged: &'a Judged,
    pub(super) signals: Option<&'a Signals>,
    pub(super) prompt: &'a str,
    pub(super) base: &'a RoutingState,
    pub(super) excluded: &'a HashSet<(ProviderId, String)>,
    pub(super) attempts: u32,
    pub(super) now: u64,
}

impl Engine {
    pub(super) fn choose(&mut self, input: ChooseInput<'_>) -> Result<Chosen, NoRoute> {
        let ChooseInput { judged, signals, prompt, base, excluded, attempts, now } = input;
        // Cleared first, so no path out of here leaves an earlier attempt's snapshot behind.
        self.git_snapshot = None;
        if let Judged::Legacy(want) = judged {
            // Routing off still answers to the privacy policy (local-only, block rules): the
            // turn goes to Claude, so Claude must be allowed to have it.
            let claude_only = catalog::Usable::all_of([ProviderId::Claude]);
            let catch_ups = self.privacy_catch_ups(&claude_only);
            let turn = TurnFacts {
                root: &self.dir,
                prompt,
                catch_ups: &catch_ups,
                task: None,
                attached: &self.images,
                pinned: true,
                attempts,
            };
            self.guards.privacy_allows(ProviderId::Claude, &turn, now).map_err(|why| NoRoute::Refused { why })?;
            return Ok(self.legacy_choice(want, base));
        }
        let usable = self.runtime.usable(now).without(excluded);
        // A team pin that can take turns at all (provider logged in, not cooling down, not
        // failed over; a pinned model known and still usable); one that cannot is reported
        // like any pin, before the turn starts. Once an attempt on it has failed, though, the
        // turn fails over like an automatic one: the pin is the repository's default, and
        // ending the turn would throw away the work of a retry the person did not opt out of.
        let team_pin_reachable = match (&self.pin, self.pin_origin) {
            (Pin::Provider(p), PinOrigin::Team) => usable.has_provider(*p).then_some(*p),
            (Pin::Model(p, id), PinOrigin::Team) => {
                catalog::find(*p, id).is_some_and(|spec| usable.contains(spec)).then_some(*p)
            }
            _ => None,
        };
        let team_pin_failed = attempts > 0 && team_pin_reachable.is_none() && self.pin_origin == PinOrigin::Team && !matches!(self.pin, Pin::Auto);
        let catch_ups = self.privacy_catch_ups(&usable);
        let turn = TurnFacts {
            root: &self.dir,
            prompt,
            catch_ups: &catch_ups,
            task: signals.map(|s| s.task),
            attached: &self.images,
            pinned: !matches!(self.pin, Pin::Auto) && self.pin_origin == PinOrigin::User,
            attempts,
        };
        let restricted = self.guards.restrict(&usable, &turn).map_err(|why| NoRoute::Refused { why })?;
        if attempts == 0 {
            self.guards.audit_blocked(&restricted.blocked, now);
        }
        let (usable, rule_notes) = (restricted.usable, restricted.notes);
        let context_tokens = (self.sessions.context_tokens_estimate() as u64).max(self.last_context_tokens);
        let plan = |p: ProviderId| self.runtime.plan_utilization(p);
        let input = RouteInput {
            signals,
            prompt_words: prompt.split_whitespace().count(),
            pin: &self.pin,
            usable: &usable,
            allow: &self.runtime.allow,
            overrides: &self.runtime.overrides,
            context_tokens,
            plan_utilization: &plan,
            now_ms: now,
        };
        // A team pin is the repository's default, not the person's choice: when their rules
        // took its provider away, or it cannot serve this task, the turn routes automatically
        // and the reason says why.
        let mut state = base.clone();
        let (decision, input, team_note) = match team_pin_reachable {
            Some(p) if !usable.has_provider(p) => {
                let note = format!("{} pin to {} skipped: your routing rules exclude {p}", crate::turn::application::team_rules::FILE, self.pin);
                let auto = RouteInput { pin: &Pin::Auto, ..input };
                (routing::decide(&mut state, &auto)?, auto, Some(note))
            }
            Some(_) => match routing::decide(&mut state, &input) {
                Err(NoRoute::PinUnusable { what }) => {
                    state = base.clone();
                    let auto = RouteInput { pin: &Pin::Auto, ..input };
                    (routing::decide(&mut state, &auto)?, auto, Some(format!("{} pin skipped: {what}", crate::turn::application::team_rules::FILE)))
                }
                decided => (decided?, input, Some(format!("pinned by {}", crate::turn::application::team_rules::FILE))),
            },
            None if team_pin_failed => {
                let note = format!("{} pin to {} skipped: it failed this turn", crate::turn::application::team_rules::FILE, self.pin);
                let auto = RouteInput { pin: &Pin::Auto, ..input };
                (routing::decide(&mut state, &auto)?, auto, Some(note))
            }
            None => (routing::decide(&mut state, &input)?, input, None),
        };
        let outcome = self.plan_orchestration(signals, &decision, &input, base);
        let (spec, reason, mcp) = self.apply_orchestration(outcome, &decision);
        let notes: Vec<String> = team_note.into_iter().chain(rule_notes).collect();
        let reason = if notes.is_empty() { reason } else { format!("{reason}; {}", notes.join("; ")) };
        let facts = RouteFacts {
            provider: spec.provider.to_string(),
            model: spec.id.to_string(),
            task: decision.task.to_string(),
            reason: reason.clone(),
            switched_provider: decision.switched_provider,
            dropped_tier: decision.dropped_tier,
        };
        Ok(Chosen { spec, reason, facts, state, legacy_tier: None, mcp })
    }

    /// What each usable provider that is behind would be caught up with (the same text
    /// prompt_for sends, git status included), for the privacy check; only worth building
    /// when a rule could match it. The git status it read is kept, so what was checked is
    /// what is sent.
    fn privacy_catch_ups(&mut self, usable: &catalog::Usable) -> Vec<(ProviderId, String)> {
        if self.guards.privacy.rule_count() == 0 {
            return Vec::new();
        }
        let behind: Vec<ProviderId> =
            crate::shared::ALL_PROVIDERS.into_iter().filter(|p| self.sessions.missed_turns(*p) > 0 && usable.has_provider(*p)).collect();
        if behind.is_empty() {
            return Vec::new();
        }
        let git = (self.git_status)(&self.dir);
        let catch_ups = behind.into_iter().filter_map(|p| self.sessions.catch_up(p, &git, CATCH_UP_TOKENS).map(|t| (p, t))).collect();
        self.git_snapshot = Some(git);
        catch_ups
    }

    /// Routing off: Claude only, a `/model` pin of a Claude model or else the
    /// upgrade-now, downgrade-slowly tier rule. Availability is not consulted; this is
    /// the old behaviour on purpose.
    fn legacy_choice(&mut self, want: &CodeWant, base: &RoutingState) -> Chosen {
        let ignored = match &self.pin {
            Pin::Provider(p) | Pin::Model(p, _) if *p != ProviderId::Claude => {
                format!(" (pin {} ignored: routing is off, claude only)", self.pin)
            }
            _ => String::new(),
        };
        let pinned = match &self.pin {
            Pin::Model(ProviderId::Claude, id) => find(ProviderId::Claude, id),
            _ => None,
        };
        let (spec, why) = match pinned {
            Some(spec) => match self.pin_origin {
                PinOrigin::User => (spec, "pinned with /model".to_string()),
                PinOrigin::Team => (spec, format!("pinned by {}", crate::turn::application::team_rules::FILE)),
            },
            None => {
                let (tier, why) = self.sticky.decide(want, self.downgrade_after);
                (legacy_spec(tier), why)
            }
        };
        let legacy_tier = LEGACY_TIERS.iter().position(|id| *id == spec.id);
        let reason = format!("{} {} — {why}{ignored}", spec.provider, spec.id);
        let facts = RouteFacts {
            provider: spec.provider.to_string(),
            model: spec.id.to_string(),
            reason: reason.clone(),
            ..RouteFacts::default()
        };
        Chosen { spec, reason, facts, state: base.clone(), legacy_tier, mcp: None }
    }

    // ------------------------------------------------------------------ running
}

pub(super) fn legacy_spec(tier: usize) -> &'static ModelSpec {
    let id = LEGACY_TIERS[tier.min(LEGACY_TIERS.len() - 1)];
    find(ProviderId::Claude, id).expect("legacy tiers are catalog models")
}
