//! Answering a running turn's tool approvals through the [`Ui`].

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use crate::backend::{self, Approver, Decision, Event, PermissionRequest};
use crate::approval::application::extension_hooks as hooks;
use crate::approval::application::extension_hooks::Hooks;
use crate::approval::domain::mode::Mode;
use crate::approval::domain::always_plan;
use crate::approval::domain::permission_request;
use crate::approval::domain::permission_rule;
use crate::approval::domain::grantable::Places;
use crate::turn::application::cancel::Cancel;
use crate::turn::application::ports::{settle, settled_decision, Ui};

/// The UI as the event callback and the approver share it; they never run at once.
pub(super) type SharedUi<'a> = Mutex<&'a mut (dyn Ui + 'a)>;

pub(super) fn lock_ui<'g, 'a>(ui: &'g SharedUi<'a>) -> std::sync::MutexGuard<'g, &'a mut (dyn Ui + 'a)> {
    ui.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Answers a running turn's tool approvals through the [`Ui`]: decides what "always" would
/// save, asks, saves it when the person says "always", tells the UI how it ended, and stops
/// the turn when they interrupt the question.
pub(super) struct EngineApprover<'a> {
    pub(super) ui: &'a SharedUi<'a>,
    pub(super) cancel: &'a Cancel,
    /// The project folder, where "always" saves its rule.
    pub(super) dir: &'a Path,
    /// The turn's mode: [`Mode::Auto`] allows tool requests without asking.
    pub(super) mode: Mode,
    /// `pre_tool` hooks may refuse a call before anyone is asked.
    pub(super) hooks: &'a Arc<Hooks>,
    /// `CODE_ALLOWED_TOOLS`: calls these rules allow are not asked about.
    pub(super) allowed_tools: &'a [String],
    /// The rules the person already saved, and where an "always" answer is saved.
    pub(super) rules: &'a dyn crate::approval::application::ports::RememberedRules,
    /// The project, home and temp folders and the disk they are on, for judging grants.
    pub(super) places: Places,
}

impl EngineApprover<'_> {
    async fn answer(&self, req: &PermissionRequest) -> Decision {
        if let Some(outcome) = hooks::runner::refusal_for(self.hooks, &req.tool, &req.summary).await {
            for warning in &outcome.warnings {
                lock_ui(self.ui).event(&Event::Hint(warning.clone()));
            }
            if let Some(reason) = outcome.refused {
                lock_ui(self.ui).event(&Event::Hint(format!("a hook refused {}: {reason}", req.tool)));
                return Decision::Deny { reason };
            }
        }
        let places = &self.places;
        // Every rule this call needs was saved before ("always"): not asked again, in any mode
        // that asks at all. Plan mode never reaches here for writes (Claude Code refuses them).
        if self.rules.remembered(&req.tool, &req.input, req.blocked_path.as_deref(), places).is_some() {
            lock_ui(self.ui).event(&Event::Hint(permission_request::remembered_allowed(&req.summary)));
            return Decision::Allow;
        }
        // What Claude Code is told with `--allowedTools`, for the CLIs that ask jev-code instead.
        // Not in plan mode, where Claude Code would still refuse anything that changes things.
        if self.mode != Mode::Plan && permission_rule::allowed_by(self.allowed_tools, &req.tool, req.input.str_field("command")) {
            lock_ui(self.ui).event(&Event::Hint(permission_request::allowed_by_setting(&req.summary)));
            return Decision::Allow;
        }
        if self.mode == Mode::Auto {
            lock_ui(self.ui).event(&Event::Hint(permission_request::auto_allowed(&req.summary)));
            return Decision::Allow;
        }
        let offer = always_plan::always_plan(&req.tool, &req.input, req.blocked_path.as_deref(), places);
        // The question goes up under the lock; the wait for the person does not hold it, so
        // the turn's own events can still be shown, and dropping this future ends the wait.
        let question = lock_ui(self.ui).ask_permission(req, &offer);
        let asked = question.await;
        // A UI that cannot ask: nothing was answered, so there is nothing to settle.
        if let Some(Decision::Deny { reason }) = &asked {
            if reason == backend::NO_ONE_TO_ASK {
                lock_ui(self.ui).event(&Event::Hint(permission_request::denied_without_asking(&req.tool)));
                return Decision::Deny { reason: reason.clone() };
            }
        }
        let settled = settle(self.rules, asked, &offer, self.dir);
        lock_ui(self.ui).permission_settled(req, &settled);
        settled_decision(&settled).unwrap_or_else(|| {
            self.cancel.cancel();
            Decision::Deny { reason: "the user interrupted the turn".into() }
        })
    }
}

impl Approver for EngineApprover<'_> {
    fn decide<'a>(&'a self, req: &'a PermissionRequest) -> std::pin::Pin<Box<dyn std::future::Future<Output = Decision> + Send + 'a>> {
        Box::pin(self.answer(req))
    }

    fn withdrawn(&self, req: &PermissionRequest) {
        lock_ui(self.ui).permission_withdrawn(req);
    }
}
