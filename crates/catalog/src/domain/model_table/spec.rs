use jev_shared::ProviderId;

/// Rough capability and price band. Ordered so `Light < Everyday < Top`, which lets
/// [`pick`] tell when a fallback landed on a weaker model than the chain wanted.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Tier {
    Light,
    Everyday,
    Top,
}

/// How using a model is paid for. A model the user's plan does not cover must not be
/// picked silently, because it spends real money rather than a subscription quota.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Billing {
    /// Included in a flat plan (Claude Pro/Max, ChatGPT, ...).
    Subscription,
    /// Extra pay-as-you-go credits, e.g. Fable on Claude Pro (included on Max).
    Credits,
    /// An API key billed per token (DeepSeek).
    PerToken,
}

/// One model the router knows about.
#[derive(Debug, PartialEq, Eq)]
pub struct ModelSpec {
    pub provider: ProviderId,
    pub id: &'static str,
    pub tier: Tier,
    pub billing: Billing,
    /// Context window; a session larger than this cannot be sent to the model.
    pub context_tokens: u64,
    /// Only tried when Jev flags the turn very hard, because these cost the most.
    pub very_hard_only: bool,
}
