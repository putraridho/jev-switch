//! The engine's one stable surface for other programs (docs/architecture/DDD.md 4.4,
//! docs/dashboard/DESIGN.md 5.1). Re-exports only; `tests/embed_api.rs` pins every name
//! and signature, so a change that breaks Flashboard fails an engine test.

pub use jev_approval::domain::always_plan::{Offer, Settled};
pub use jev_approval::domain::mode::Mode;
pub use jev_backend::{
    Decision, ElicitationAnswer, ElicitationRequest, Event, Finish, PermissionRequest, PlanUsage,
};
pub use jev_providers::domain::availability::codex::parse_codex_login;
pub use jev_providers::domain::login_hints::login_command;
pub use jev_routing::domain::decide::Pin;
pub use jev_session::domain::turn_status::TurnStatus;
pub use jev_shared::infrastructure::env_file::{env_file_path, parse as parse_env};
pub use jev_shared::infrastructure::jsonl_log::state_dir;
pub use jev_shared::ProviderId;
pub use jev_turn::application::cancel::Cancel;
pub use jev_turn::application::engine::Engine;
pub use jev_turn::application::ports::{Answer, ElicitationReply, SteerFilter, Ui};
pub use jev_turn::application::turn_report::TurnReport;
pub use jev_turn::domain::failover::{Failover, NoRouteInfo};
pub use jev_turn::domain::route_facts::RouteFacts;
pub use jev_turn::infrastructure::desktop_engine::{
    build_desktop_engine, catalog_models, provider_readiness, DesktopEngine, ProviderReadiness,
};
