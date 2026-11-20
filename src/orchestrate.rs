//! The planner side of cross-provider orchestration: when does a turn get a PLANNER
//! that hands pieces to cheaper workers, and how is the delegate tool attached to it.
//! (The worker side, picking workers and running them in worktrees, is
//! [`crate::delegate`].)
//!
//! Everything that decides is pure, so each rule is a unit test and the engine only
//! sequences them. Orchestration is ON by default in a session and what `/orchestrate
//! on|off` sets is kept for the next one (a one-shot `-p` stays off unless `JEV_ORCHESTRATE=on`):
//! a planner plus workers costs more than one model on a small task, so it only triggers when
//! Jev says the request splits into pieces.
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
use crate::router::{Orchestration, ORCH_TRIGGER_P};

/// The MCP server name; the tools reach the planner as `mcp__jev__delegate` etc.
pub const MCP_NAME: &str = "jev";
/// The binary that serves the delegate tool (`src/bin/delegate_mcp.rs`).
pub const DELEGATE_BIN: &str = "delegate_mcp";
/// The name `scripts/install.sh` gives it next to `jev-code`; [`Setup::detect`] finds both.
pub const DELEGATE_BIN_INSTALLED: &str = "jev-delegate-mcp";

/// Whether a prompt is orchestrated, as far as the numbers go.
#[derive(Debug, PartialEq)]
pub enum Gate {
    Trigger,
    BelowCutoff,
    /// Multi-part enough, but a sequence of dependent steps (the veto): refused whatever the
    /// cutoff is.
    Vetoed,
    /// Multi-part enough, but normal routing would not have used a top model.
    SingleModelRoute,
}

/// Whether a prompt is orchestrated: `multi_part` at the cutoff or above, not a sequence
/// (the veto holds for every cutoff) and, when `require_opus`, a normal route of Opus
/// (tier 2) or higher. A route Jev couldn't judge (`None`) doesn't veto. Lives here (not in
/// the gateway binary) so the gateway and the multi-provider engine apply literally the
/// same rule.
pub fn orch_gate(judgment: Option<&Orchestration>, cutoff: f64, route_tier: Option<usize>, require_opus: bool) -> Gate {
    let Some(j) = judgment.filter(|j| j.multi_part_p >= cutoff) else { return Gate::BelowCutoff };
    if j.vetoed {
        return Gate::Vetoed;
    }
    match route_tier {
        Some(t) if require_opus && t < 2 => Gate::SingleModelRoute,
        _ => Gate::Trigger,
    }
}

impl Orchestration {
    /// Whether the judgment alone clears the gate at `cutoff`: multi-part enough and not a
    /// sequence. [`orch_gate`] with no routing requirement, so the gateway (which asks the
    /// routing question only for a prompt that passes) and the engine use one rule.
    pub fn passes(&self, cutoff: f64) -> bool {
        orch_gate(Some(self), cutoff, None, false) == Gate::Trigger
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
    judgment: Option<&Orchestration>,
    normal: &'static ModelSpec,
    tool_ready: bool,
    mcp_providers: &[ProviderId],
    restricted_route: impl FnOnce(&[ProviderId]) -> Option<&'static ModelSpec>,
) -> Outcome {
    if !enabled
        || orch_gate(judgment, ORCH_TRIGGER_P, Some(route_tier_of(normal)), true) != Gate::Trigger
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

/// What `JEV_ORCHESTRATE` says: `on` (also `1`, `true`, `yes`), `off` (also `0`, `false`, `no`),
/// or nothing when it is unset or says something else.
pub fn env_choice(env: Option<&str>) -> Option<bool> {
    match env?.trim().to_ascii_lowercase().as_str() {
        "on" | "1" | "true" | "yes" => Some(true),
        "off" | "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

/// Whether orchestration starts on. A session: what the last one left (`/orchestrate`), else
/// `JEV_ORCHESTRATE`, else on. A one-shot (`-p`) has no one to ask and costs what it costs
/// in a script, so there only an explicit `JEV_ORCHESTRATE=on` turns it on.
pub fn start(saved: Option<bool>, env: Option<&str>, session: bool) -> bool {
    if session {
        saved.or(env_choice(env)).unwrap_or(true)
    } else {
        env_choice(env) == Some(true)
    }
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
        let bin = match crate::envfile::lookup("JEV_DELEGATE_BIN") {
            Some(path) => Some(PathBuf::from(path)).filter(|p| p.is_file()),
            None => std::env::current_exe().ok().and_then(|exe| beside(&exe)),
        };
        Some(Self { bin, config_dir })
    }

    /// The attachment for one planner, or `None` when the binary was not found.
    pub fn attachment(&self, project: &Path, planner: &ModelSpec) -> Option<McpAttachment> {
        let bin = self.bin.as_ref()?;
        Some(attachment(bin, project, planner, &self.config_dir, std::process::id()))
    }
}

/// The delegate binary next to `exe`: [`DELEGATE_BIN`] (where `cargo build` puts it)
/// or [`DELEGATE_BIN_INSTALLED`] (where `scripts/install.sh` puts it), whichever exists.
pub fn beside(exe: &Path) -> Option<PathBuf> {
    [DELEGATE_BIN, DELEGATE_BIN_INSTALLED].iter().map(|name| exe.with_file_name(name)).find(|p| p.is_file())
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

pub use crate::backend::claude::write_mcp_config;

/// Put in front of the prompt of an orchestrated turn so the planner uses the tool
/// instead of doing every piece itself. It says nothing about models: Jev picks them.
pub const PLANNER_NOTE: &str = "[orchestration] This task splits into separate pieces. You have a `delegate` tool (MCP server \
`jev`) that runs a piece in a cheaper worker model. It returns a task id at once and the worker runs in the background: start \
every independent piece first, keep working on your own part, then call `wait` to collect the results. Give each a complete, \
self-contained brief (a worker does not see this conversation). Mark research and lookups `read_only`: they \
read your folder and return a report. An editing worker runs in its own git worktree from the last commit (it does not see your \
uncommitted edits); when its tests pass and its changes apply cleanly, `wait` auto-merges them into your tree and you get its report \
and file list (`undo` takes them back out). Otherwise you get the diff with the reason: files listed as not shown need \
`show_diff`, `merge` only what you verified (pass `files` for a subset), and `discard` the rest. Pass `review: true` for risky \
changes you want to read first. Do the integration work yourself. You cannot choose the worker's model.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{find, models};
    use ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

    fn spec(p: ProviderId, id: &str) -> &'static ModelSpec {
        find(p, id).unwrap()
    }

    /// A parallel request Jev scored `p` on `multi_part`.
    fn parallel(p: f64) -> Orchestration {
        Orchestration::new(p, 0.1)
    }

    #[test]
    fn orchestrate_only_what_normal_routing_would_send_to_opus() {
        let hot = parallel(0.98);
        assert_eq!(orch_gate(Some(&hot), 0.7, Some(2), true), Gate::Trigger, "multi-part and Opus-bound");
        assert_eq!(orch_gate(Some(&hot), 0.7, Some(3), true), Gate::Trigger, "Fable-bound counts");
        assert_eq!(orch_gate(Some(&hot), 0.7, Some(1), true), Gate::SingleModelRoute, "Sonnet would do it alone");
        assert_eq!(orch_gate(Some(&hot), 0.7, Some(1), false), Gate::Trigger, "ORCH_REQUIRE_OPUS=off keeps the old rule");
        assert_eq!(orch_gate(Some(&hot), 0.7, None, true), Gate::Trigger, "no routing judgment: don't veto");
        assert_eq!(orch_gate(Some(&parallel(0.5)), 0.7, Some(2), true), Gate::BelowCutoff);
        assert_eq!(orch_gate(None, 0.7, Some(2), true), Gate::BelowCutoff, "Jev failed the multi-part check");
    }

    #[test]
    fn the_sequence_veto_holds_whatever_the_cutoff_is() {
        let sequence = Orchestration::new(0.95, 0.9);
        assert!(sequence.vetoed);
        // A cutoff the old 0.30 cap could not stay under: the veto used to silently fail here.
        for cutoff in [0.7, 0.3, 0.2, 0.05, 0.0] {
            assert_eq!(orch_gate(Some(&sequence), cutoff, Some(2), true), Gate::Vetoed, "cutoff {cutoff}");
            assert_eq!(orch_gate(Some(&sequence), cutoff, Some(2), false), Gate::Vetoed, "cutoff {cutoff}, any route");
        }
        // Below the cutoff it is just below the cutoff, as it would be without a veto.
        assert_eq!(orch_gate(Some(&Orchestration::new(0.1, 0.9)), 0.7, Some(2), true), Gate::BelowCutoff);
    }

    #[test]
    fn the_judgment_alone_passes_only_when_multi_part_enough_and_not_a_sequence() {
        // The predicate the gateway asks before it pays for the routing question.
        assert!(parallel(0.7).passes(0.7), "at the cutoff");
        assert!(!parallel(0.69).passes(0.7), "under it");
        assert!(!Orchestration::new(0.95, 0.9).passes(0.7), "a sequence, however multi-part");
        assert!(!Orchestration::new(0.95, 0.9).passes(0.0), "at any cutoff");
        assert!(Orchestration::judged(0.95, None).passes(0.7), "no sequential answer: no veto information, multi_part decides");
        for (j, cutoff) in [(parallel(0.8), 0.7), (parallel(0.5), 0.7), (Orchestration::new(0.9, 0.95), 0.2)] {
            assert_eq!(j.passes(cutoff), orch_gate(Some(&j), cutoff, Some(2), true) == Gate::Trigger, "one rule for both");
        }
    }

    #[test]
    fn route_tier_matches_the_gateways_numbering() {
        assert_eq!(route_tier_of(spec(Claude, "claude-haiku-5-5")), 0);
        assert_eq!(route_tier_of(spec(Claude, "claude-sonnet-5-5")), 1);
        assert_eq!(route_tier_of(spec(Claude, "claude-opus-5-5")), 2);
        assert_eq!(route_tier_of(spec(Claude, "claude-fable-5-2")), 3);
    }

    #[test]
    fn a_session_starts_with_orchestration_on_unless_a_choice_says_otherwise() {
        // The env names a choice only with a clear word.
        assert_eq!(env_choice(None), None);
        assert_eq!(env_choice(Some("")), None);
        assert_eq!(env_choice(Some("maybe")), None);
        for on in ["on", "ON", " 1 ", "true", "yes"] {
            assert_eq!(env_choice(Some(on)), Some(true), "{on}");
        }
        for off in ["off", "0", "False", "no"] {
            assert_eq!(env_choice(Some(off)), Some(false), "{off}");
        }
        // A session: kept choice, else the env, else on.
        assert!(start(None, None, true));
        assert!(start(None, Some("maybe"), true));
        assert!(!start(None, Some("off"), true));
        assert!(!start(Some(false), Some("on"), true), "what the person set last wins over the env");
        assert!(start(Some(true), Some("off"), true));
        // A one-shot only turns it on when told to, and never reads a kept choice.
        assert!(!start(Some(true), None, false));
        assert!(!start(None, Some("off"), false));
        assert!(start(None, Some("on"), false));
    }

    const MCP: [ProviderId; 3] = [Claude, DeepSeek, Codex];

    fn choose(enabled: bool, p: f64, normal: &'static ModelSpec, ready: bool, alt: Option<&'static ModelSpec>) -> Outcome {
        choose_planner(enabled, Some(&parallel(p)), normal, ready, &MCP, |keep| {
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
        let sequence = Orchestration::new(0.95, 0.9);
        assert_eq!(choose_planner(true, Some(&sequence), opus, true, &MCP, |_| None), Outcome::Normal, "a sequence is never orchestrated");
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
    fn the_delegate_binary_is_found_beside_jev_code_under_either_name() {
        let dir = std::env::temp_dir().join(format!("jev-orch-beside-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("jev-code");
        assert_eq!(beside(&exe), None, "nothing installed");
        std::fs::write(dir.join(DELEGATE_BIN_INSTALLED), "").unwrap();
        assert_eq!(beside(&exe), Some(dir.join(DELEGATE_BIN_INSTALLED)), "the installed name");
        std::fs::write(dir.join(DELEGATE_BIN), "").unwrap();
        assert_eq!(beside(&exe), Some(dir.join(DELEGATE_BIN)), "the cargo build name wins");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_planner_note_does_not_let_the_planner_pick_models() {
        assert!(PLANNER_NOTE.contains("cannot choose the worker's model"));
    }
}
