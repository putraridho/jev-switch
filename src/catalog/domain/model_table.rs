//! Which models exist, and which of them may answer a given kind of task.
//!
//! Jev (a separate judgment service) decides what kind of task a turn is; this
//! module never judges a task. It only maps a [`TaskType`] to an ordered list of
//! models, and then filters that list by what the user can actually use right now.
//! Everything here is a pure function so the whole rule set is testable without a
//! network, a CLI or a clock.
//!
//! The core rule: a turn is NEVER routed to a model the user can't use. [`pick`]
//! enforces it, so a caller cannot forget to.
//!
//! The parts live in child modules; the items callers need are re-exported here so
//! the many importers keep one stable path.

mod chains;
mod models;
mod overrides;
mod pick;
mod spec;
mod task_type;
mod usable;

pub use chains::{chain_with, default_chain};
pub use models::{allowed_for, find, models, MODEL_CHEAP, MODEL_MID, MODEL_STRONG, MODEL_TOP};
pub use overrides::{Overrides, RawChain, RawEntry};
pub use pick::{pick, Allowances, Pick};
pub use spec::{Billing, ModelSpec, Tier};
pub use task_type::TaskType;
pub use usable::Usable;

#[cfg(test)]
mod tests;
