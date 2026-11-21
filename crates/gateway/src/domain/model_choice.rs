//! The alias Claude Code asks for, and the four real models the gateway can swap in for it.

use jev_catalog::domain::model_table::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG, MODEL_TOP};

pub const ALIAS: &str = "jev-code";
pub const TIERS: [&str; 4] = [MODEL_CHEAP, MODEL_MID, MODEL_STRONG, MODEL_TOP];
