//! The planner side of cross-provider orchestration: when does a turn get a PLANNER
//! that hands pieces to cheaper workers, and how is the delegate tool attached to it.
//! (The worker side, picking workers and running them in worktrees, is
//! [`crate::delegate`].)
//!
//! Everything that decides is pure, so each rule is a unit test and the engine only
//! sequences them. Orchestration is OFF by default and per session (`/orchestrate
//! on|off`, or `JEV_ORCHESTRATE=on` at start): a planner plus workers costs more than
//! one model on a small task, so it must be asked for.
//!
//! Hard rules:
//! - it triggers only when Jev says the request splits into pieces (`multi_part` at
//!   [`ORCH_TRIGGER_P`] or above) AND the normal route for the turn would already be a
//!   Top-tier model ([`orch_gate`], the one rule the Claude-only gateway also uses);
//! - the planner is only chosen among providers whose CLI can have the delegate tool
//!   attached ([`crate::backend::Backend::supports_mcp`]); if none can, the turn runs
//!   normally and the `[route]` line says why.

use std::path::{Path, PathBuf};

use crate::backend::{McpAttachment, ProviderId};
use crate::catalog::{ModelSpec, Tier};
use crate::router::ORCH_TRIGGER_P;

/// The MCP server name; the tools reach the planner as `mcp__jev__delegate` etc.
pub const MCP_NAME: &str = "jev";
/// The binary that serves the delegate tool (`src/bin/delegate_mcp.rs`).
pub const DELEGATE_BIN: &str = "delegate_mcp";

/// Whether a prompt is orchestrated, as far as the numbers go.
#[derive(Debug, PartialEq)]
pub enum Gate {
    Trigger,
    BelowCutoff,
    /// Multi-part enough, but normal routing would not have used a top model.
    SingleModelRoute,
}

/// Whether a prompt is orchestrated: `multi_part` at the cutoff or above and, when
/// `require_opus`, a normal route of Opus (tier 2) or higher. A route Jev couldn't judge
/// (`None`) doesn't veto. Lives here (not in the gateway binary) so the gateway and
/// the multi-provider engine apply literally the same rule.
pub fn orch_gate(multi_part: Option<f64>, cutoff: f64, route_tier: Option<usize>, require_opus: bool) -> Gate {
    if !multi_part.is_some_and(|p| p >= cutoff) {
        return Gate::BelowCutoff;
    }
    match route_tier {
        Some(t) if require_opus && t < 2 => Gate::SingleModelRoute,
        _ => Gate::Trigger,
    }
}

/// The gateway's tier number for a catalog model: 0 light, 1 everyday, 2 top, 3 a
/// top model reserved for very hard work (Fable, Astra, Carbon).
pub fn route_tier_of(spec: &ModelSpec) -> usize {
    match (spec.tier, spec.very_hard_only) {
        (Tier::Light, _) => 0,
        (Tier::Everyday, _) => 1,
        (Tier::Top, false) => 2,
        (Tier::Top, true) => 3,
    }
}

/// What to do with a turn that is a candidate for orchestration.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Not orchestrated, and nothing worth saying: off, below the cutoff, or a route that
    /// would not use a top model anyway.
    Normal,
    /// Run the turn on this planner with the delegate tool attached.
    Planner(&'static ModelSpec),
    /// Should have been orchestrated but cannot be; the text goes on the `[route]` line.
    Unavailable(String),
}

/// Decide the planner. `normal` is the model the turn would run on anyway;
/// `mcp_providers` are the providers that can take the delegate tool;
/// `restricted_route` re-runs the routing with only those providers usable and returns
/// its model (injected because it needs the engine's state).
pub fn choose_planner(
    enabled: bool,
    multi_part_p: Option<f64>,
    normal: &'static ModelSpec,
    tool_ready: bool,
    mcp_providers: &[ProviderId],
    restricted_route: impl FnOnce(&[ProviderId]) -> Option<&'static ModelSpec>,
) -> Outcome {
    if !enabled
        || orch_gate(multi_part_p, ORCH_TRIGGER_P, Some(route_tier_of(normal)), true) != Gate::Trigger
    {
        return Outcome::Normal;
    }
    if !tool_ready {
        return Outcome::Unavailable(format!("the {DELEGATE_BIN} binary was not found next to jev-code"));
    }
    if mcp_providers.contains(&normal.provider) {
        return Outcome::Planner(normal);
    }
    match restricted_route(mcp_providers).filter(|s| s.tier == Tier::Top) {
        Some(spec) => Outcome::Planner(spec),
        None => Outcome::Unavailable(format!(
            "{} cannot take the delegate tool and no provider that can ({}) has a usable top model for this task",
            normal.provider,
            names(mcp_providers)
        )),
    }
}

fn names(providers: &[ProviderId]) -> String {
    providers.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", ")
}

/// The `/orchestrate` switch at start: only an explicit yes turns it on, because it is
/// off by default (`JEV_ORCHESTRATE=on`, also `1`, `true`, `yes`).
pub fn enabled_from_env(env: Option<&str>) -> bool {
    env.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "on" | "1" | "true" | "yes"))
}

/// Where the delegate tool lives and where its per-run config goes.
#[derive(Debug, Clone)]
pub struct Setup {
    /// The `delegate_mcp` binary, if it was found.
    pub bin: Option<PathBuf>,
    /// Where the MCP config file for Claude Code is written.
    pub config_dir: PathBuf,
}

impl Setup {
    /// `JEV_DELEGATE_BIN`, else `delegate_mcp` beside the running executable (where
    /// `cargo build` and `install.sh` put it). The config goes in the state dir.
    pub fn detect() -> Option<Self> {
        let config_dir = crate::log::state_dir()?;
        let bin = std::env::var_os("JEV_DELEGATE_BIN")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok().map(|e| e.with_file_name(DELEGATE_BIN)))
            .filter(|p| p.is_file());
        Some(Self { bin, config_dir })
    }

    /// The attachment for one planner, or `None` when the binary was not found.
    pub fn attachment(&self, project: &Path, planner: &ModelSpec) -> Option<McpAttachment> {
        let bin = self.bin.as_ref()?;
        Some(attachment(bin, project, planner, &self.config_dir, std::process::id()))
    }
}

/// The server entry for one planner. `instance` (the process id) keeps two `jev-code`
/// sessions from overwriting each other's config file.
pub fn attachment(bin: &Path, project: &Path, planner: &ModelSpec, config_dir: &Path, instance: u32) -> McpAttachment {
    McpAttachment {
        name: MCP_NAME.to_string(),
        command: bin.to_path_buf(),
        args: vec![
            "--dir".into(),
            project.to_string_lossy().into_owned(),
            "--planner".into(),
            format!("{}:{}", planner.provider, planner.id),
        ],
        config_path: config_dir.join(format!("mcp-delegate-{instance}.json")),
    }
}

/// Write the JSON file Claude Code's `--mcp-config` reads. Harmless for other CLIs.
pub fn write_mcp_config(mcp: &McpAttachment) -> std::io::Result<()> {
    let json = crate::backend::claude::mcp_config_json(mcp);
    std::fs::write(&mcp.config_path, json.to_string())
}

/// Put in front of the prompt of an orchestrated turn so the planner uses the tool
/// instead of doing every piece itself. It says nothing about models: Jev picks them.
pub const PLANNER_NOTE: &str = "[orchestration] This task splits into separate pieces. You have a `delegate` tool (MCP server \
`jev`) that runs a piece in a cheaper worker model, in its own git worktree, and returns a summary and diff. Delegate the \
independent pieces (call it several times in parallel), each with a complete, self-contained brief (a worker sees only the \
brief and the last commit, not your uncommitted edits or this conversation). Review every returned diff: files the result lists \
as not shown need `show_diff` (page it until you have read them), and `merge` refuses anything you have not seen. `merge` only \
what you verified (pass `files` to take a subset), `discard` the rest, and do the integration work yourself. You cannot choose \
the worker's model.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{find, models};
    use ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

    fn spec(p: ProviderId, id: &str) -> &'static ModelSpec {
        find(p, id).unwrap()
    }

    #[test]
    fn orchestrate_only_what_normal_routing_would_send_to_opus() {
        assert_eq!(orch_gate(Some(0.98), 0.7, Some(2), true), Gate::Trigger, "multi-part and Opus-bound");
        assert_eq!(orch_gate(Some(0.98), 0.7, Some(3), true), Gate::Trigger, "Fable-bound counts");
        assert_eq!(orch_gate(Some(0.98), 0.7, Some(1), true), Gate::SingleModelRoute, "Sonnet would do it alone");
        assert_eq!(orch_gate(Some(0.98), 0.7, Some(1), false), Gate::Trigger, "ORCH_REQUIRE_OPUS=off keeps the old rule");
        assert_eq!(orch_gate(Some(0.98), 0.7, None, true), Gate::Trigger, "no routing judgment: don't veto");
        assert_eq!(orch_gate(Some(0.5), 0.7, Some(2), true), Gate::BelowCutoff);
        assert_eq!(orch_gate(None, 0.7, Some(2), true), Gate::BelowCutoff, "Jev failed the multi-part check");
    }

    #[test]
    fn route_tier_matches_the_gateways_numbering() {
        assert_eq!(route_tier_of(spec(Claude, "claude-haiku-5-5")), 0);
        assert_eq!(route_tier_of(spec(Claude, "claude-sonnet-5-5")), 1);
        assert_eq!(route_tier_of(spec(Claude, "claude-opus-5-5")), 2);
        assert_eq!(route_tier_of(spec(Claude, "claude-fable-5-2")), 3);
    }

    #[test]
    fn orchestration_is_off_unless_the_env_says_on() {
        assert!(!enabled_from_env(None));
        assert!(!enabled_from_env(Some("")));
        assert!(!enabled_from_env(Some("off")));
        assert!(!enabled_from_env(Some("maybe")));
        for on in ["on", "ON", " 1 ", "true", "yes"] {
            assert!(enabled_from_env(Some(on)), "{on}");
        }
    }

    const MCP: [ProviderId; 3] = [Claude, DeepSeek, Codex];

    fn choose(enabled: bool, p: f64, normal: &'static ModelSpec, ready: bool, alt: Option<&'static ModelSpec>) -> Outcome {
        choose_planner(enabled, Some(p), normal, ready, &MCP, |keep| {
            assert_eq!(keep, MCP, "the restricted route only sees MCP-capable providers");
            alt
        })
    }

    #[test]
    fn a_top_model_with_mcp_support_plans_when_multi_part_and_enabled() {
        let opus = spec(Claude, "claude-opus-5-5");
        assert_eq!(choose(true, 0.9, opus, true, None), Outcome::Planner(opus));
        assert_eq!(choose(false, 0.9, opus, true, None), Outcome::Normal, "off by default");
        assert_eq!(choose(true, 0.5, opus, true, None), Outcome::Normal, "not multi-part");
        let sonnet = spec(Claude, "claude-sonnet-5-5");
        assert_eq!(choose(true, 0.9, sonnet, true, None), Outcome::Normal, "normal route would not use a top model");
    }

    #[test]
    fn planner_selection_skips_providers_without_mcp_support() {
        // Gemini and Grok cannot take the tool, so a Top route onto them looks for another planner.
        for no_mcp in [spec(Gemini, "gemini-4-argon"), spec(Grok, "grok-4.7")] {
            let alt = spec(Codex, "gpt-6.1-sol");
            assert_eq!(choose(true, 0.9, no_mcp, true, Some(alt)), Outcome::Planner(alt));
            match choose(true, 0.9, no_mcp, true, None) {
                Outcome::Unavailable(why) => assert!(why.contains("cannot take the delegate tool"), "{why}"),
                other => panic!("{other:?}"),
            }
            // A restricted route that lands on a non-Top model is not a planner.
            let weak = spec(Codex, "gpt-6.1-terra");
            assert!(matches!(choose(true, 0.9, no_mcp, true, Some(weak)), Outcome::Unavailable(_)));
        }
        // Whatever the restricted route returned, a planner never comes from a provider outside the list.
        for m in models().iter().filter(|m| m.tier == Tier::Top) {
            if let Outcome::Planner(p) = choose(true, 0.9, m, true, Some(spec(Claude, "claude-opus-5-5"))) {
                assert!(MCP.contains(&p.provider));
            }
        }
    }

    #[test]
    fn a_missing_tool_binary_is_reported_not_ignored() {
        let opus = spec(Claude, "claude-opus-5-5");
        assert!(matches!(choose(true, 0.9, opus, false, None), Outcome::Unavailable(w) if w.contains(DELEGATE_BIN)));
        assert_eq!(choose(false, 0.9, opus, false, None), Outcome::Normal, "off stays silent");
    }

    #[test]
    fn attachment_names_the_project_and_planner() {
        let opus = spec(Claude, "claude-opus-5-5");
        let a = attachment(Path::new("/bin/delegate_mcp"), Path::new("/repo"), opus, Path::new("/state"), 42);
        assert_eq!(a.name, "jev");
        assert_eq!(a.args, ["--dir", "/repo", "--planner", "claude:claude-opus-5-5"]);
        assert_eq!(a.config_path, Path::new("/state/mcp-delegate-42.json"));
        let setup = Setup { bin: None, config_dir: "/state".into() };
        assert!(setup.attachment(Path::new("/repo"), opus).is_none());
    }

    #[test]
    fn the_planner_note_does_not_let_the_planner_pick_models() {
        assert!(PLANNER_NOTE.contains("cannot choose the worker's model"));
    }
}
