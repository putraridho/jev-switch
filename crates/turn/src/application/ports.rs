//! What a turn talks to: the [`Ui`] it shows progress on and asks questions through.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use jev_backend::{self as backend, BackendFactory, Decision, Event, McpAttachment, PermissionRequest, ProviderId, TurnError, TurnOutcome};
use jev_session::application::ports::SessionStore;
use jev_shared::application::host::Host;
use jev_approval::domain::always_plan::{Offer, Settled};
use crate::domain::failover::{Failover, NoRouteInfo};
use crate::domain::route_facts::RouteFacts;
use crate::domain::usage_ledger::Book;

/// Whether a steered message should still be sent when the turn comes to take it.
pub type SteerFilter = Box<dyn Fn(&str) -> bool + Send + Sync>;

/// Where a turn shows its progress. The binary prints; tests record.
/// `Send` because the runner calls back from inside its own future.
pub trait Ui: Send {
    /// The routing decision, one line, before the turn runs.
    fn route(&mut self, line: &str);
    /// Streamed progress: tool calls, assistant text, permission hints.
    fn event(&mut self, e: &Event);
    /// Anything else the user should know: failover, errors, "nothing is usable".
    fn note(&mut self, line: &str);

    // The structured callbacks below carry the facts behind a line, so a UI can word and
    // style it itself. Each one defaults to the plain-text callback above, so a UI that
    // only prints strings keeps working unchanged.

    /// The routing decision with its parts. Called instead of [`Ui::route`] by the engine.
    fn route_facts(&mut self, facts: &RouteFacts) {
        self.route(&facts.reason);
    }

    /// An attempt failed: either the turn moves on to the next model
    /// ([`Failover::will_retry`]) or this is the final error.
    fn failover(&mut self, f: &Failover) {
        self.note(f.text);
    }

    /// No model can take the turn; nothing was started. `text` is the plain-words message.
    fn no_route(&mut self, _info: &NoRouteInfo, text: &str) {
        self.note(text);
    }

    /// The user stopped the turn.
    fn cancelled(&mut self, text: &str) {
        self.note(text);
    }

    /// Messages the person types while the next attempt runs, for a backend that can add them
    /// to the turn ([`backend::Backend::steerable`]). The sender lives as long as the UI wants
    /// them taken; dropping the receiver (the attempt is over) tells it to stop. `None`: this
    /// UI has no input during a turn, or does not forward it.
    fn steering(&mut self) -> Option<tokio::sync::mpsc::UnboundedReceiver<String>> {
        None
    }

    /// Whether a message offered through [`Ui::steering`] should still be sent when the turn
    /// comes to take it: `false` once the person withdrew it (took it back out of the queue to
    /// edit it). `None`: nothing is ever withdrawn.
    fn steer_wanted(&mut self) -> Option<SteerFilter> {
        None
    }

    /// A tool call waits for the person's answer. `offer` is what [`Decision::AllowAlways`]
    /// would save, or why "always" is not offered: the engine decided it, the UI only shows it
    /// (and offers the choice only when it is `Ok`). The question is put on screen by this call; the returned [`Answer`] resolves with what the person said
    /// (`None` means they interrupted with Ctrl-C, which stops the turn). Dropping the
    /// answer before it resolves abandons the question (the CLI withdrew it): the UI stops
    /// listening for a key at once, and [`Ui::permission_withdrawn`] follows. A UI that
    /// cannot ask keeps this default, so a run with no terminal denies at once and never
    /// waits for an answer nobody gives.
    fn ask_permission(&mut self, _req: &PermissionRequest, _offer: &Offer) -> Answer {
        Box::pin(std::future::ready(Some(Decision::Deny { reason: backend::NO_ONE_TO_ASK.to_string() })))
    }

    /// How the question for `req` ended, once the engine has saved what "always" saves (or
    /// failed to): the answer line. Called once per answered question, after the save, so
    /// the screen only ever says what is true. A UI without an answer line still hears about
    /// a save that failed.
    fn permission_settled(&mut self, _req: &PermissionRequest, settled: &Settled) {
        if let Settled::NotSaved(e) = settled {
            self.note(&format!("[permissions] {e}; allowed this once, nothing was saved"));
        }
    }

    /// The question asked for `req` was dropped unanswered: take it off the screen. Called
    /// once per such question, never for one that was answered.
    fn permission_withdrawn(&mut self, _req: &PermissionRequest) {}

    /// A server asks for information. This is never settled by tool permission policy.
    fn elicit(&mut self, _req: &backend::ElicitationRequest) -> ElicitationReply {
        Box::pin(std::future::ready(backend::ElicitationAnswer::Cancel))
    }

    fn elicitation_withdrawn(&mut self, _req: &backend::ElicitationRequest) {}
}

/// The person's answer to a question, not yet given. It owns what it needs, so the UI is
/// free to print while the person thinks.
pub type ElicitationReply = std::pin::Pin<Box<dyn std::future::Future<Output = backend::ElicitationAnswer> + Send>>;

pub type Answer = std::pin::Pin<Box<dyn std::future::Future<Output = Option<Decision>> + Send>>;

/// Settle a question's answer: the one place "always" is saved. The offer is what was shown;
/// "always" saves exactly that (rule and folder in one write) or, when the file cannot be
/// written, allows once and says so. An "always" with nothing on offer is a plain yes.
pub fn settle(rules: &dyn jev_approval::application::ports::RememberedRules, answer: Option<Decision>, offer: &Offer, project: &Path) -> Settled {
    match answer {
        None => Settled::Stopped,
        Some(Decision::Allow) => Settled::Allowed,
        Some(Decision::AllowAlways) => match offer {
            Ok(always) => match rules.save_always(project, always) {
                Ok(_) => Settled::Remembered(always.clone()),
                Err(e) => Settled::NotSaved(e),
            },
            Err(_) => Settled::Allowed,
        },
        Some(Decision::Deny { reason }) => Settled::Denied(reason),
    }
}

/// What the CLI is told for a settled question; `None` when the person stopped the turn.
pub fn settled_decision(settled: &Settled) -> Option<Decision> {
    match settled {
        Settled::Allowed | Settled::NotSaved(_) => Some(Decision::Allow),
        Settled::Remembered(_) => Some(Decision::AllowAlways),
        Settled::Denied(reason) => Some(Decision::Deny { reason: reason.clone() }),
        Settled::Stopped => None,
    }
}

/// What one attempt adds to the turn log, besides what the outcome already carries.
pub struct AttemptRow<'a> {
    pub provider: ProviderId,
    pub model: &'a str,
    pub session: Option<&'a str>,
    pub reason: &'a str,
    /// Jev's numbers for the turn.
    pub jev: &'a Value,
    pub result: &'a Result<TurnOutcome, TurnError>,
    pub wall_ms: u64,
    pub attempt: u32,
    pub orchestrated: bool,
    pub cancelled: bool,
    pub notes: &'a [String],
}

/// Where a turn's attempts are logged. Real: `turn_log::FileTurnLog`.
pub trait TurnLog: Send + Sync {
    /// Append the row of one attempt to the log at `path`.
    fn append(&self, path: &str, row: &AttemptRow<'_>);
}

/// `git status --short` of a folder.
pub type GitStatus = Box<dyn Fn(&Path) -> String + Send + Sync>;

/// Writes the MCP config file a planner CLI reads.
pub type McpWriter = Box<dyn Fn(&McpAttachment) -> std::io::Result<()> + Send + Sync>;

/// Where the privacy audit log is kept: one record per prompt sent or kept back, never the
/// prompt itself. Real: `turn::infrastructure::privacy_audit::FilePrivacyAudit`.
pub trait PrivacyAudit: std::fmt::Debug + Send + Sync {
    /// Append `record` to the log at `path`; never fails the turn.
    fn record(&self, path: &Path, record: &crate::domain::privacy::audit::AuditRecord);
    /// The last `count` records of the log at `path`, oldest first; unreadable lines are skipped.
    fn tail(&self, path: &Path, count: usize) -> Vec<crate::domain::privacy::audit::AuditRecord>;
}

/// The usage ledger file in the state folder.
pub const USAGE_LEDGER_FILE: &str = "usage-ledger.json";

/// Where the usage of every process of a session is kept (see [`crate::domain::usage_ledger`]).
/// Real: `turn::infrastructure::usage_ledger_file::FileUsageLedger`; tests may use the in-memory one.
pub trait UsageLedger: std::fmt::Debug + Send + Sync {
    /// Run `change` on the ledger at `path` as one step no other handle (thread or process) can
    /// interleave with: it sees the latest state and what it leaves is what the next one sees.
    /// `false` when the ledger could not be read or written, and then `change` may not have run.
    fn transact(&self, path: &Path, change: &mut dyn FnMut(&mut Book)) -> bool;
    /// The current state without changing it; `None` when it cannot be read.
    fn read(&self, path: &Path) -> Option<Book>;
}

/// Discover named MCP servers and the providers configured to use each.
pub type McpDiscovery = Box<dyn Fn(&Path) -> Vec<crate::domain::mcp_access::McpAccess> + Send + Sync>;

/// The outer adapters passed by the composition root to the turn engine.
pub struct Adapters {
    pub mcp_discovery: McpDiscovery,
    pub backend_factory: BackendFactory,
    pub git_status: GitStatus,
    pub turn_log: Arc<dyn TurnLog>,
    pub idle_timeout: Duration,
    pub write_mcp: McpWriter,
    /// Settings, the clock, the state directory and private file writes.
    pub host: Arc<dyn Host>,
    /// Where saved sessions are kept.
    pub sessions: Arc<dyn SessionStore>,
    /// What grant checks ask about the disk.
    pub path_facts: Arc<dyn jev_approval::domain::grantable::PathFacts>,
    /// The rules a person already saved, and where an "always" answer is saved.
    pub remembered_rules: Arc<dyn jev_approval::application::ports::RememberedRules>,
    /// Where the privacy audit log is written.
    pub privacy_audit: Arc<dyn PrivacyAudit>,
    /// Where session usage is shared with the workers' processes.
    pub usage_ledger: Arc<dyn UsageLedger>,
    /// Whether another process is still running (reservations of dead ones are reaped).
    pub liveness: Arc<dyn jev_orchestration::application::ports::Liveness>,
}
