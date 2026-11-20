//! Giving a turn a planner when the orchestration gate opens.

use crate::backend::{McpAttachment, ProviderId};
use crate::catalog::domain::model_table::ModelSpec;
use crate::approval::domain::mode::Mode;
use crate::orchestration::domain::orchestration_gate as orchestrate;
use crate::orchestration::domain::orchestration_gate::Outcome;
use crate::routing::domain::decide as routing;
use crate::routing::domain::decide::{RouteDecision, RouteInput, RoutingState, Signals};
use super::Engine;

impl Engine {
    /// Providers whose CLI can have the delegate tool attached. A backend is cheap to
    /// build and says it itself, so no list of providers is kept here to drift.
    fn mcp_providers(&self) -> Vec<ProviderId> {
        crate::shared::ALL_PROVIDERS.into_iter().filter(|p| (self.backend_factory)(*p).supports_mcp()).collect()
    }

    /// Should this turn have a planner? The decision is [`orchestrate::choose_planner`];
    /// this supplies what it reads. The planner is the normal route's model when its CLI
    /// can take the tool, else the best Top model among the providers that can (the same
    /// routing, with only those providers usable, so a `/model` pin is still respected).
    pub(super) fn plan_orchestration(
        &self,
        signals: Option<&Signals>,
        normal: &RouteDecision,
        input: &RouteInput,
        base: &RoutingState,
    ) -> Outcome {
        let tool_ready = self.orch_setup.as_ref().is_some_and(|s| s.bin.is_some());
        // Plan mode changes nothing, and the planner's `merge` would change the working
        // tree: no planner then (`/review`, `/compact` and a team limit of plan rely on it).
        let enabled = self.orchestrate && self.mode != Mode::Plan;
        let mcp = if enabled { self.mcp_providers() } else { Vec::new() };
        orchestrate::choose_planner(
            enabled,
            signals.map(|s| &s.orchestration),
            normal.spec,
            tool_ready,
            &mcp,
            |keep| {
                let restricted = input.usable.only_providers(keep);
                let mut state = base.clone();
                routing::decide(&mut state, &RouteInput { usable: &restricted, ..*input }).ok().map(|d| d.spec)
            },
        )
    }

    /// The model, the `[route]` line and the tool to attach once orchestration has had its say.
    pub(super) fn apply_orchestration(&self, outcome: Outcome, normal: &RouteDecision) -> (&'static ModelSpec, String, Option<McpAttachment>) {
        match outcome {
            Outcome::Normal => (normal.spec, normal.reason.clone(), None),
            Outcome::Unavailable(why) => (normal.spec, format!("{}; orchestration skipped: {why}", normal.reason), None),
            Outcome::Planner(spec) => {
                let mcp = self.orch_setup.as_ref().and_then(|s| s.attachment(&self.dir, spec));
                let moved = if spec == normal.spec {
                    String::new()
                } else {
                    format!(" ({} cannot take the delegate tool)", normal.spec.provider)
                };
                let reason = format!("{}; orchestrating: {} {} plans{moved}, Jev picks the workers", normal.reason, spec.provider, spec.id);
                (spec, reason, mcp)
            }
        }
    }
}
