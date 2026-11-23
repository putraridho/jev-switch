//! The ACP transport of Antigravity (`agy acp`) and of Grok Build (`grok agent stdio`). Print
//! mode soft-denies tools needing approval; ACP keeps stdin open and sends those questions
//! through the host approver.
//! Protocol: https://agentclientprotocol.com/protocol/v1/tool-calls

use std::collections::HashMap;

use serde_json::{json, Value};

mod permission;
mod tool_row;
#[cfg(test)]
mod tests;
mod update;

use super::gemini_cli::stream_events::finish_or_empty;
use crate::{Event, Measurement, TurnRequest, Usage};
use jev_approval::domain::mode::Mode;

/// Which CLI is on the other end. Only what genuinely differs branches on it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Agent {
    /// Advertises its modes (`default`, `accept-edits`, `plan`); only advertised ones are used.
    #[default]
    Antigravity,
    /// Advertises none but takes `default` and `plan` (checked against grok 1.0.92 on
    /// 2026-11-21: `acceptEdits`, `dontAsk` and `bypassPermissions` are refused as unknown).
    /// With no accept-edits mode an edit is asked about too, so an edit request is passed on
    /// as `Edit` with its path, for the approver to judge.
    Grok,
}

impl Agent {
    fn label(self) -> &'static str {
        match self {
            Agent::Antigravity => "antigravity ACP",
            Agent::Grok => "grok ACP",
        }
    }
}

#[derive(Default)]
pub(crate) struct Acp {
    agent: Agent,
    request: Option<TurnRequest>,
    session: Option<String>,
    text: String,
    usage: Usage,
    outbound: Vec<String>,
    calls: HashMap<String, Value>,
    /// Calls whose tool row was sent: a call is announced once it says what it does.
    announced: std::collections::HashSet<String>,
    permissions: HashMap<String, Value>,
    mode: String,
    active: bool,
}

fn rpc(id: u64, method: &str, params: Value) -> String {
    format!("{}\n", json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
}

impl Acp {
    pub fn for_agent(agent: Agent) -> Self {
        Self { agent, ..Self::default() }
    }

    pub fn start(&mut self, req: &TurnRequest) -> String {
        *self = Self {
            agent: self.agent,
            request: Some(req.clone()),
            usage: Usage { measurement: Measurement::Unavailable, ..Usage::default() },
            ..Self::default()
        };
        rpc(
            1,
            "initialize",
            json!({"protocolVersion":1, "clientCapabilities":{},
            "clientInfo":{"name":"jev-code", "version":env!("CARGO_PKG_VERSION")}}),
        )
    }

    pub fn drain(&mut self) -> Vec<String> {
        std::mem::take(&mut self.outbound)
    }

    pub fn parse(&mut self, ev: &Value) -> Vec<Event> {
        if let Some(method) = ev["method"].as_str() {
            return match method {
                "session/update" => self.update(&ev["params"]),
                "session/request_permission" => self.permission(ev),
                _ if ev.get("id").is_some() => {
                    self.outbound.push(format!(
                        "{}\n",
                        json!({"jsonrpc":"2.0", "id":ev["id"],
                        "error":{"code":-32601,"message":"client method not supported"}})
                    ));
                    vec![]
                }
                _ => vec![],
            };
        }
        let Some(id) = ev["id"].as_u64() else { return vec![] };
        let label = self.agent.label();
        if let Some(error) = ev.get("error") {
            // Grok puts the reason in `data` ("Rate limited" / "subscription:free-usage-exhausted:
            // You've used all the included free usage ..."): the classifier needs it.
            let message = error["message"].as_str().unwrap_or("request failed");
            let detail = error["data"].as_str().filter(|d| !d.trim().is_empty()).map(|d| format!(": {d}")).unwrap_or_default();
            return vec![Event::Failed { message: format!("{label}: {message}{detail}") }];
        }
        let Some(req) = &self.request else { return vec![] };
        let result = &ev["result"];
        match id {
            1 => {
                if result["protocolVersion"] != 1 {
                    return vec![Event::Failed { message: format!("{label} does not support protocol version 1") }];
                }
                let mut params = json!({"cwd":req.dir, "mcpServers":[]});
                let method = if let Some(session) = &req.session_id {
                    params["sessionId"] = json!(session);
                    self.session = Some(session.clone());
                    "session/load"
                } else {
                    "session/new"
                };
                self.outbound.push(rpc(2, method, params));
            }
            2 => {
                if let Some(id) = result["sessionId"].as_str() {
                    self.session = Some(id.into());
                }
                if self.session.is_none() {
                    return vec![Event::Failed { message: format!("{label} returned no session ID") }];
                }
                if self.agent == Agent::Grok {
                    self.mode = if req.mode == Mode::Plan { "plan" } else { "default" }.to_string();
                    self.outbound.push(rpc(3, "session/set_model", json!({"sessionId":self.session,"modelId":req.model})));
                    return vec![];
                }
                // Use only advertised modes. Auto decisions stay in jev-code, as with
                // Claude: accept edits, then ask the host about shell commands.
                let aliases: &[&str] = match req.mode {
                    Mode::Ask => &["default", "ask"],
                    Mode::Edit | Mode::Auto => &["accept-edits", "auto_edit", "autoEdit"],
                    Mode::Plan => &["plan"],
                };
                let modes = result["modes"]["availableModes"].as_array();
                let selected =
                    aliases.iter().find(|alias| modes.is_some_and(|ms| ms.iter().any(|m| m["id"] == **alias)));
                let Some(mode) = selected else {
                    return vec![Event::Failed { message: format!("{label} does not advertise a compatible {} mode", req.mode) }];
                };
                self.mode = mode.to_string();
                self.outbound.push(rpc(3, "session/set_model", json!({"sessionId":self.session,"modelId":req.model})));
            }
            3 => self.outbound.push(rpc(4, "session/set_mode", json!({"sessionId":self.session,"modeId":self.mode}))),
            4 => {
                // session/load replays history; none of it is this turn's answer.
                self.active = true;
                self.text.clear();
                self.calls.clear();
                self.announced.clear();
                self.outbound.push(rpc(
                    5,
                    "session/prompt",
                    json!({"sessionId":self.session,
                    "prompt":[{"type":"text","text":crate::prompt_with_images(req)}]}),
                ));
            }
            5 => {
                if result["stopReason"] != "end_turn" {
                    return vec![Event::Failed {
                        message: format!("{label} stopped: {}", result["stopReason"].as_str().unwrap_or("unknown reason")),
                    }];
                }
                // A command still reported as running when the turn ends was left in the
                // background (Antigravity moves slow commands there): nothing will report on it.
                let running = self.calls.values().filter(|c| c["kind"] == "execute" && c["status"] == "in_progress").count();
                let mut out: Vec<Event> = jev_shared::domain::background::notice(running, false).filter(|_| running > 0).map(Event::Hint).into_iter().collect();
                out.push(finish_or_empty(&self.text, self.session.clone(), self.usage, ""));
                return out;
            }
            _ => {}
        }
        vec![]
    }

}
