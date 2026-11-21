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
    jev_router::orchestration::interface::mcp::compose::run(Box::new(|state, dir| {
        std::sync::Arc::new(jev_router::turn::infrastructure::worker_policy::GuardedWorkers(jev_router::turn::application::guards::Guards::load(&jev_router::shared::infrastructure::system_host::SystemHost, Some(state), dir)))
    }))
    .await
}
