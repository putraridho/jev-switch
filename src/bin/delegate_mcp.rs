//! delegate_mcp: the MCP server that gives an orchestration PLANNER its `delegate`,
//! `wait`, `show_diff`, `merge`, `discard` and `undo` tools. A planner CLI (Claude Code, Codex)
//! starts it as a stdio child: newline-delimited JSON-RPC 2.0, one message per line.
//!
//! The tool has NO model parameter. Which worker model runs a piece is Jev's decision
//! (see `jev_router::orchestration::application::delegator::Delegator`); anything the planner says about models is ignored.
//!
//!   delegate_mcp [--dir <project dir>] [--planner <provider:model>]
//!
//! Provider probing and Jev are set up lazily on the first `delegate` call, so the
//! handshake is instant. Stdout is the protocol channel: this binary prints nothing else
//! there. The protocol lives in `src/orchestration/interface/mcp/`.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    jev_router::shared::infrastructure::platform::ensure_home();
    jev_router::orchestration::interface::mcp::compose::run(Box::new(|state, dir| {
        let guards = jev_router::turn::application::guards::Guards::load(&jev_router::shared::infrastructure::system_host::SystemHost, std::sync::Arc::new(jev_router::turn::infrastructure::privacy_audit::FilePrivacyAudit), Some(state), dir);
        // The workers' paid-use checks follow how each CLI is funded, as the planner's do.
        let mut guards = guards;
        guards.set_fundings(jev_router::providers::infrastructure::funding_probe::detect(&jev_router::providers::infrastructure::funding_probe::SystemAccountFacts));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        // The planner's session is in the shared usage ledger: its spend counts here, and
        // each worker attempt reserves its budget there.
        let guards = guards.with_ledger(jev_router::turn::application::usage_account::UsageAccount::worker(
            std::sync::Arc::new(jev_router::turn::infrastructure::usage_ledger_file::FileUsageLedger),
            std::sync::Arc::new(jev_router::orchestration::infrastructure::process_liveness::ProcLiveness),
            state.join(jev_router::turn::application::ports::USAGE_LEDGER_FILE),
            dir,
            now,
        ));
        std::sync::Arc::new(jev_router::turn::infrastructure::worker_policy::GuardedWorkers::new(guards))
    }))
    .await
}
