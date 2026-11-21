//! Antigravity's ACP transport. Print mode soft-denies tools needing approval;
//! ACP keeps stdin open and sends those questions through the host approver.
//! Protocol: https://agentclientprotocol.com/protocol/v1/tool-calls

use std::collections::HashMap;

use serde_json::{json, Value};

mod permission;
mod tool_row;
#[cfg(test)]
mod tests;
mod update;

use super::gemini_cli::stream_events::finish_or_empty;
use crate::backend::{Event, TurnRequest, Usage};
use crate::approval::domain::mode::Mode;

#[derive(Default)]
pub(super) struct Acp {
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
    pub fn start(&mut self, req: &TurnRequest) -> String {
        *self = Self { request: Some(req.clone()), ..Self::default() };
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
        if let Some(error) = ev.get("error") {
            return vec![Event::Failed {
                message: format!("antigravity ACP: {}", error["message"].as_str().unwrap_or("request failed")),
            }];
        }
        let Some(req) = &self.request else { return vec![] };
        let result = &ev["result"];
        match id {
            1 => {
                if result["protocolVersion"] != 1 {
                    return vec![Event::Failed {
                        message: "antigravity ACP does not support protocol version 1".into(),
                    }];
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
                    return vec![Event::Failed { message: "antigravity ACP returned no session ID".into() }];
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
                    return vec![Event::Failed {
                        message: format!("antigravity ACP does not advertise a compatible {} mode", req.mode),
                    }];
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
                    "prompt":[{"type":"text","text":crate::backend::prompt_with_images(req)}]}),
                ));
            }
            5 => {
                if result["stopReason"] != "end_turn" {
                    return vec![Event::Failed {
                        message: format!(
                            "antigravity ACP stopped: {}",
                            result["stopReason"].as_str().unwrap_or("unknown reason")
                        ),
                    }];
                }
                // A command still reported as running when the turn ends was left in the
                // background (Antigravity moves slow commands there): nothing will report on it.
                let running = self.calls.values().filter(|c| c["kind"] == "execute" && c["status"] == "in_progress").count();
                let mut out: Vec<Event> = crate::shared::domain::background::notice(running, false).filter(|_| running > 0).map(Event::Hint).into_iter().collect();
                out.push(finish_or_empty(&self.text, self.session.clone(), self.usage, ""));
                return out;
            }
            _ => {}
        }
        vec![]
    }

}
