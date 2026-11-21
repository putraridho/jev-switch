//! The file form of a plan reading (`crate::providers::infrastructure::plan_store`).

use serde::{Deserialize, Serialize};

use crate::backend::domain::plan_usage::PlanUsage;

#[derive(Serialize, Deserialize)]
pub struct PlanUsageFile {
    pub window: String,
    pub utilization: f64,
    pub resets_at: Option<u64>,
}

impl From<PlanUsageFile> for PlanUsage {
    fn from(f: PlanUsageFile) -> Self {
        PlanUsage { window: f.window, utilization: f.utilization, resets_at: f.resets_at }
    }
}

impl From<&PlanUsage> for PlanUsageFile {
    fn from(p: &PlanUsage) -> Self {
        PlanUsageFile { window: p.window.clone(), utilization: p.utilization, resets_at: p.resets_at }
    }
}
