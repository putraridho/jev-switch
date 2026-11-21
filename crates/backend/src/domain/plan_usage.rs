//! A plan-limit window as a CLI reported it.

/// What a plan-limit window looked like when the CLI last reported it. Informational.
/// Saved across restarts in the shape of `crate::infrastructure::plan_usage_file`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanUsage {
    pub window: String,
    pub utilization: f64,
    pub resets_at: Option<u64>,
}
