//! One coding turn, end to end: ask Jev what the turn is, pick a model that is really
//! usable, bring that model up to date on what the others did, run it, and if it
//! fails in a way another provider can fix, try the next one.
//!
//! The policy lives elsewhere ([`crate::routing::domain::decide`] picks, [`crate::catalog::domain::model_table`] knows the
//! models, [`crate::session::domain::catch_up`] writes the catch-up, [`crate::providers::application::provider_registry`] knows who is
//! usable). This module only sequences them, so each step stays testable on its own
//! and the `jev-code` binary is a thin shell that prints what a [`Ui`] is handed.
//!
//! Two hard rules hold on every path:
//! - a provider that is not usable is never started (the choice always goes through
//!   [`routing::decide`] with the live usable set, also after a failure);
//! - a failed attempt never produces an answer: only a successful attempt returns text.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::turn::domain::effort::EffortSetting;
use crate::changes::infrastructure::git_working_tree::{ChangeSummary, TurnActivity};
use crate::turn::application::guards::Guards;
use crate::approval::application::extension_hooks::Hooks;
use crate::session::domain::catch_up::Sessions;
use crate::approval::domain::mode::Mode;
use crate::orchestration::domain::orchestration_gate::Setup;
use crate::providers::application::provider_registry::Runtime;
use crate::judgment::application::judge::Router;
use crate::routing::domain::decide::{Pin, PinOrigin, RoutingState, Sticky};

use crate::backend::BackendFactory;

mod accessors;
mod attempt;
mod builder;
mod change_commands;
mod choose_model;
mod finish_turn;
mod plan_orchestration;
mod run_attempt;
mod run_attempts;
mod run_turn;
mod session_commands;
#[cfg(test)]
mod tests;

/// Budget for the catch-up block put in front of a prompt after a provider switch.
const CATCH_UP_TOKENS: usize = 10_000;

type GitStatus = Box<dyn Fn(&Path) -> String + Send + Sync>;
type Clock = Box<dyn Fn() -> u64 + Send + Sync>;

pub struct Engine {
    pub runtime: Runtime,
    pub sessions: Sessions,
    pub routing: RoutingState,
    /// Privacy policy, routing rules, budget, failover policy and the session's meters.
    pub guards: Guards,
    router: Router,
    pin: Pin,
    pin_origin: PinOrigin,
    dir: PathBuf,
    enabled: bool,
    backend_factory: BackendFactory,
    git_status: GitStatus,
    now: Clock,
    idle_timeout: Duration,
    /// Show assistant text as it arrives instead of holding it until the attempt succeeds
    /// (the interactive screen: a failed attempt's text is then followed by a note saying it
    /// was cut off, which the failover line already says).
    stream_text: bool,
    turn_log: Option<String>,
    /// The last (prompt, reply), for Jev's correction question.
    previous: Option<(String, String)>,
    /// Context size the last answering CLI reported.
    last_context_tokens: u64,
    /// Legacy (routing off) stickiness.
    sticky: Sticky,
    downgrade_after: u32,
    /// `/orchestrate`: off until asked for, because a planner plus workers costs more
    /// than one model on a small task.
    orchestrate: bool,
    /// Where the delegate tool is; `None` means orchestration cannot attach it.
    orch_setup: Option<Setup>,
    /// Snapshot the working tree around every turn (see [`crate::changes::infrastructure::git_working_tree`]).
    track_changes: bool,
    /// What the last turn that started a model changed; consumed by an undo.
    last_changes: Option<ChangeSummary>,
    /// Every turn of this session that changed files, oldest first: the checkpoints that
    /// `/diff --turn`, `/diff --all` and `/rewind` work on.
    change_history: Vec<ChangeSummary>,
    /// Images for the next turn (dropped or pasted paths, see [`crate::turn::application::attach`]); sent with
    /// every attempt of that turn, then dropped.
    images: Vec<PathBuf>,
    /// What the tools of the turn in progress did (all attempts), to tell its changes from
    /// other processes'.
    turn_activity: TurnActivity,
    /// Where saved sessions live (the state directory); `None` means nothing is saved.
    persist: Option<PathBuf>,
    /// Where `/budget`, `/privacy` and `/failover` read and save their settings; the state
    /// directory even for a one-shot that keeps no session.
    settings: Option<PathBuf>,
    /// The `git status` the privacy check saw for this attempt's catch-ups, reused when the
    /// catch-up is sent so the text checked is the text that leaves.
    git_snapshot: Option<String>,
    /// The file this session saves to: chosen at its first save, or the one `/resume` took
    /// over. `/new` clears it, so the next save starts another file and the old one stays.
    session_file: Option<PathBuf>,
    /// What a turn may do without asking (`/mode`, Shift-Tab).
    mode: Mode,
    /// `/effort`: the reasoning effort passed to CLIs that have the control.
    effort: EffortSetting,
    /// Shell commands run on tool calls and turn events ([`crate::approval::application::extension_hooks`]).
    hooks: Arc<Hooks>,
    /// `CODE_ALLOWED_TOOLS`, split into rules: what a turn may do without asking, on every CLI.
    allowed_tools: Vec<String>,
}

pub(crate) struct SummaryContext<'a> {
    pub engine: &'a mut Engine,
    sessions: Sessions,
    routing: RoutingState,
    sticky: Sticky,
    previous: Option<(String, String)>,
    last_context_tokens: u64,
    last_changes: Option<ChangeSummary>,
    images: Vec<PathBuf>,
    persist: Option<PathBuf>,
    mode: Mode,
    track_changes: bool,
}

impl Drop for SummaryContext<'_> {
    fn drop(&mut self) {
        self.engine.sessions = std::mem::take(&mut self.sessions);
        self.engine.routing = std::mem::take(&mut self.routing);
        self.engine.sticky = std::mem::take(&mut self.sticky);
        self.engine.previous = self.previous.take();
        self.engine.last_context_tokens = self.last_context_tokens;
        self.engine.last_changes = self.last_changes.take();
        self.engine.images = std::mem::take(&mut self.images);
        self.engine.persist = self.persist.take();
        self.engine.mode = self.mode;
        self.engine.track_changes = self.track_changes;
    }
}

impl Engine {
    /// Borrow a fresh, read-only CLI context without changing the saved conversation,
    /// even if the summary fails or its future is dropped. Usage is still metered.
    pub(crate) fn summary_context(&mut self) -> SummaryContext<'_> {
        SummaryContext {
            sessions: std::mem::take(&mut self.sessions),
            routing: std::mem::take(&mut self.routing),
            sticky: std::mem::take(&mut self.sticky),
            previous: self.previous.take(),
            last_context_tokens: std::mem::take(&mut self.last_context_tokens),
            last_changes: self.last_changes.take(),
            images: std::mem::take(&mut self.images),
            persist: self.persist.take(),
            mode: std::mem::replace(&mut self.mode, Mode::Plan),
            track_changes: std::mem::replace(&mut self.track_changes, false),
            engine: self,
        }
    }
}
