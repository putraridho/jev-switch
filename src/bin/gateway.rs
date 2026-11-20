//! jev-gateway: a local Anthropic-format gateway that adds a "jev-code" model
//! to the normal Claude Code interface.
//!
//! Claude Code points `ANTHROPIC_BASE_URL` here. Every request is forwarded to
//! Anthropic unchanged (headers, body, streamed response), with one exception:
//! when the request's `model` is `jev-code`, the gateway swaps in a real model.
//! It decides once per user prompt (Claude Code's `x-claude-code-prompt-id` hint
//! header groups a turn's requests), using code mode's policy: Sonnet by default,
//! Opus for confident reasoning, strong security terms, or edits to auth code,
//! with the same upgrade-now / downgrade-slowly stickiness. Small background
//! requests (titles, summaries) go to Haiku, adapted to what Haiku accepts; forks of
//! the conversation and compaction stay on the turn's model; subagents run on Sonnet. Three JSON-lines logs: routing decisions, one usage
//! record per /v1/messages request, and one record per user turn (see docs/guide.md, "Usage log").
//!
//!   jev-gateway        (normally started by the `jev-claude` launcher)
//!
//! Env: JEV_GATEWAY_ADDR (127.0.0.1:18780), JEV_GATEWAY_UPSTREAM
//! (https://api.anthropic.com), CODE_STRONG_THRESHOLD (0.85), DOWNGRADE_AFTER (2),
//! JEV_GATEWAY_LOG (~/.local/state/jev-router/gateway.jsonl), JEV_USAGE_LOG
//! (usage.jsonl), JEV_TURNS_LOG (turns.jsonl). WORKER_CHEAP_P (unset: workers never get Haiku), ORCH_REQUIRE_OPUS (on). Fable tier: JEV_FABLE (on; "off" disables),
//! FABLE_THRESHOLD (0.90). Orchestrate mode (docs/guide.md): JEV_ORCH
//! (on; "off" disables), ORCH_TRIGGER_P (0.70), ORCH_PLANNER (claude-opus-5-5),
//! JEV_ORCH_LOG (orch.jsonl). JEV_GATEWAY_MAX_INFLIGHT (32),
//! JEV_GATEWAY_ALLOW_REMOTE (1 to allow non-loopback binding).

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    jev_router::gateway::interface::http::compose::run().await
}
