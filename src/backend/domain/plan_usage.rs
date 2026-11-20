//! A plan-limit window as a CLI reported it.

/// What a plan-limit window looked like when the CLI last reported it. Informational.
/// Serializable so the last reading survives a restart (`crate::providers::infrastructure::plan_store`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanUsage {
    pub window: String,
    pub utilization: f64,
    pub resets_at: Option<u64>,
}
