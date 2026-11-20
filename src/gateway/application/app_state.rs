//! The gateway's shared state: its router, upstream client, sessions, log paths and
//! orchestrate settings.

use reqwest::Client;
use tokio::sync::Mutex;

use crate::gateway::application::session_state::Sessions;
use crate::judgment::application::judge::Router;

pub(crate) struct App {
    pub(crate) router: Router,
    pub(crate) http: Client,
    pub(crate) upstream: String,
    pub(crate) downgrade_after: u32,
    pub(crate) sessions: Mutex<Sessions>,
    pub(crate) log_path: Option<String>,
    pub(crate) usage_path: Option<String>,
    pub(crate) turns_path: Option<String>,
    pub(crate) orch: Orch,
}

/// Orchestrate mode settings (see docs/guide.md, "Orchestrate mode").
pub(crate) struct Orch {
    /// `JEV_ORCH=off` turns the trigger off for every session.
    pub(crate) enabled: bool,
    /// `ORCH_TRIGGER_P`: Jev's raw `multi_part` at or above which a prompt triggers. A
    /// sequence (the `sequential` veto) never triggers, whatever the cutoff.
    pub(crate) cutoff: f64,
    /// The model the main thread runs on for an orchestrated turn when Jev has no say:
    /// Opus, or `ORCH_PLANNER` if set.
    pub(crate) planner: &'static str,
    /// `ORCH_PLANNER` is set: it pins the planner and Jev does not pick it.
    pub(crate) planner_pinned: bool,
    /// `ORCH_REQUIRE_OPUS` (default on; `0` or `off` disables): orchestrate only a prompt
    /// that normal routing would have sent to Opus. When Sonnet alone would do the work,
    /// orchestration only adds an expensive planner (15x a Sonnet run on a small task).
    pub(crate) require_opus: bool,
    pub(crate) log_path: Option<String>,
}
