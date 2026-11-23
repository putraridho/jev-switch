//! The real adapters an [`Engine`](crate::application::engine::Engine) is built with:
//! the session-keeping provider backends, `git status`, the JSON-lines turn log, the idle
//! timeout from the environment and the MCP config file.

use std::sync::Arc;

use jev_backend::idle_timeout_from_env;
use jev_backend::infrastructure::claude::claude_cli::mcp_config::write_mcp_config;
use jev_providers::infrastructure::backend_factory::session_backend_for;
use jev_session::infrastructure::session_file::FileSessions;
use jev_shared::infrastructure::system_host::SystemHost;
use crate::application::ports::Adapters;

use super::git_status::git_status_short;
use super::turn_log::FileTurnLog;

pub fn real() -> Adapters {
    Adapters {
        backend_factory: Box::new(session_backend_for),
        git_status: Box::new(git_status_short),
        turn_log: Arc::new(FileTurnLog),
        idle_timeout: idle_timeout_from_env(),
        write_mcp: Box::new(write_mcp_config),
        host: Arc::new(SystemHost),
        sessions: Arc::new(FileSessions),
        path_facts: Arc::new(jev_approval::infrastructure::path_facts::SystemPathFacts),
        remembered_rules: Arc::new(jev_approval::infrastructure::claude_settings_file::ClaudeSettingsFiles { host: Arc::new(SystemHost) }),
        privacy_audit: Arc::new(super::privacy_audit::FilePrivacyAudit),
        spend_ledger: Arc::new(super::spend_ledger_file::FileSpendLedger),
    }
}
