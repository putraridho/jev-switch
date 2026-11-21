//! The gateway's shared state: its router, upstream client, sessions, log paths and
//! orchestrate settings.

use reqwest::Client;
use tokio::sync::Mutex;

use crate::application::session_state::Sessions;
use jev_judgment::application::judge::Router;

pub struct App {
    pub router: Router,
    pub http: Client,
    pub upstream: String,
    pub downgrade_after: u32,
    pub sessions: Mutex<Sessions>,
    pub log_path: Option<String>,
    pub usage_path: Option<String>,
    pub turns_path: Option<String>,
    pub orch: Orch,
}

/// Orchestrate mode settings (see docs/guide.md, "Orchestrate mode").
pub struct Orch {
    /// `JEV_ORCH=off` turns the trigger off for every session.
    pub enabled: bool,
    /// `ORCH_TRIGGER_P`: Jev's raw `multi_part` at or above which a prompt triggers. A
    /// sequence (the `sequential` veto) never triggers, whatever the cutoff.
    pub cutoff: f64,
    /// The model the main thread runs on for an orchestrated turn when Jev has no say:
    /// Opus, or `ORCH_PLANNER` if set.
    pub planner: &'static str,
    /// `ORCH_PLANNER` is set: it pins the planner and Jev does not pick it.
    pub planner_pinned: bool,
    /// `ORCH_REQUIRE_OPUS` (default on; `0` or `off` disables): orchestrate only a prompt
    /// that normal routing would have sent to Opus. When Sonnet alone would do the work,
    /// orchestration only adds an expensive planner (15x a Sonnet run on a small task).
    pub require_opus: bool,
    pub log_path: Option<String>,
}
