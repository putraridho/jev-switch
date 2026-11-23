use super::models::allowed_for;
use super::{Billing, ModelSpec, TaskType, Usable};

/// Which spending the user has agreed to. Subscription models need no permission.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Allowances {
    /// Allow [`Billing::Credits`] models.
    pub credits: bool,
    /// Allow [`Billing::PerToken`] models; the caller sets this only if a key is configured.
    pub per_token: bool,
}

impl Allowances {
    fn permits(&self, billing: Billing) -> bool {
        match billing {
            Billing::Subscription => true,
            Billing::Credits => self.credits,
            Billing::PerToken => self.per_token,
        }
    }
}

/// The chosen model and how far down the chain it was.
#[derive(Debug, Clone, Copy)]
pub struct Pick {
    pub spec: &'static ModelSpec,
    /// Index in the chain; 0 means the first choice was available.
    pub rank: usize,
    /// The pick is a lower tier than this kind of work needs ([`TaskType::needed_tier`]),
    /// e.g. a hard bug landing on an everyday model because no top model is usable.
    pub dropped_tier: bool,
}

/// First chain entry that is usable, billing-allowed, allowed for the task and big
/// enough for the current session; `None` if nothing qualifies. The caller turns
/// `None` into an honest "no usable model" answer rather than guessing.
pub fn pick(
    chain: &[&'static ModelSpec],
    task: TaskType,
    usable: &Usable,
    allow: &Allowances,
    context_tokens: u64,
) -> Option<Pick> {
    let (rank, spec) = chain.iter().copied().enumerate().find(|(_, s)| {
        usable.contains(s) && allow.permits(s.billing) && allowed_for(task, s) && usable.fits(s, context_tokens)
    })?;
    Some(Pick { spec, rank, dropped_tier: spec.tier < task.needed_tier() })
}
