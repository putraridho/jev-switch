//! The engine's one stable surface for other programs (docs/architecture/DDD.md 4.4,
//! docs/dashboard/DESIGN.md 5.1). Re-exports only; `tests/embed_api.rs` pins every name
//! and signature, so a change that breaks Flashboard fails an engine test.

pub use jev_providers::domain::availability::codex::parse_codex_login;
pub use jev_providers::domain::login_hints::login_command;
pub use jev_shared::infrastructure::env_file::{env_file_path, parse as parse_env};
pub use jev_shared::infrastructure::jsonl_log::state_dir;
pub use jev_shared::ProviderId;
