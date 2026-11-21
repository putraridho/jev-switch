//! The MCP stdio server behind the orchestration planner's `delegate`, `wait`, `show_diff`,
//! `merge`, `discard` and `undo` tools: JSON-RPC framing, tool schemas, argument parsing,
//! the lazily built delegator and the stdio loop.

mod json_rpc;
mod lazy_delegator;
mod tool_arguments;
mod tool_schema;

pub mod compose;

#[cfg(test)]
mod tests;
