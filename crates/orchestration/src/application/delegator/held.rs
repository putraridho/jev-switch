//! The budget an admitted worker attempt holds: settled with its usage, released if it never
//! ran, and settled as unknown consumption if the attempt is dropped half way (the planner
//! cancelled the call), so a cancelled attempt is never counted as free.

use jev_backend::{Measurement, Usage};
use jev_catalog::domain::model_table::ModelSpec;

use crate::application::ports::{BudgetHold, WorkerPolicy};

pub(super) struct Held<'a> {
    policy: &'a dyn WorkerPolicy,
    hold: Option<BudgetHold>,
    spec: &'static ModelSpec,
}

impl<'a> Held<'a> {
    pub(super) fn new(policy: &'a dyn WorkerPolicy, hold: BudgetHold, spec: &'static ModelSpec) -> Self {
        Self { policy, hold: Some(hold), spec }
    }

    /// End the hold with what the attempt used.
    pub(super) fn settle(mut self, usage: &Usage) {
        if let Some(hold) = self.hold.take() {
            self.policy.settle(hold, self.spec.provider, self.spec.id, usage);
        }
    }

    /// End the hold of an attempt that never started: nothing was used.
    pub(super) fn release(mut self) {
        if let Some(hold) = self.hold.take() {
            self.policy.release(hold);
        }
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if let Some(hold) = self.hold.take() {
            let unknown = Usage { measurement: Measurement::Unavailable, ..Usage::default() };
            self.policy.settle(hold, self.spec.provider, self.spec.id, &unknown);
        }
    }
}
