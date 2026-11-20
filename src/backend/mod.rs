pub mod application;
pub mod domain;
pub mod infrastructure;
#[cfg(test)]
pub mod testing;

// The front door: what the rest of the crate says as `crate::backend::Name`.
pub use crate::approval::domain::permission_request::{Decision, PermissionRequest};
pub use crate::shared::{ProviderId, Usage};
pub use application::ports::*;
pub use domain::effort::Effort;
pub use domain::error_kind::ErrorKind;
pub use domain::error_text::*;
pub use domain::event::{Event, Finish};
pub use domain::mcp_attachment::McpAttachment;
pub use domain::plan_usage::PlanUsage;
pub use domain::prompt_arg::*;
pub use domain::turn_error::TurnError;
pub use domain::turn_outcome::TurnOutcome;
pub use domain::turn_request::*;
pub use infrastructure::process::child::{run_turn, run_turn_steered, run_turn_with};
pub use infrastructure::process::idle_watchdog::{idle_timeout, idle_timeout_from_env};
pub use infrastructure::process::prompt_files::{prompt_arg, prompt_arg_in, PromptFiles};
#[cfg(all(test, unix))]
pub(crate) use infrastructure::process::process_group::process_alive;
