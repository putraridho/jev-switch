//! Antigravity's ACP transport. Print mode soft-denies tools needing approval;
//! ACP keeps stdin open and sends those questions through the host approver.
//! Protocol: https://agentclientprotocol.com/protocol/v1/tool-calls

use std::collections::HashMap;

use serde_json::{json, Value};

use super::{finish_or_empty, tool_summary, Event, TurnRequest, Usage};
use crate::backend::{Decision, PermissionRequest};
use crate::mode::Mode;

#[derive(Default)]
pub(super) struct Acp {
    request: Option<TurnRequest>,
    session: Option<String>,
    text: String,
    usage: Usage,
    outbound: Vec<String>,
    calls: HashMap<String, Value>,
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
                return vec![finish_or_empty(&self.text, self.session.clone(), self.usage, "")];
            }
            _ => {}
        }
        vec![]
    }

    fn update(&mut self, params: &Value) -> Vec<Event> {
        if !self.active || params["sessionId"].as_str() != self.session.as_deref() {
            return vec![];
        }
        let update = &params["update"];
        match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => {
                if update["content"]["type"] == "text" {
                    self.text.push_str(update["content"]["text"].as_str().unwrap_or(""));
                }
            }
            Some("tool_call" | "tool_call_update") => {
                let Some(id) = update["toolCallId"].as_str() else { return vec![] };
                let first = !self.calls.contains_key(id);
                let call = self.calls.entry(id.into()).or_insert_with(|| json!({}));
                if let Some(fields) = update.as_object() {
                    for (k, v) in fields {
                        call[k] = v.clone();
                    }
                }
                if first {
                    return vec![Event::Tool(tool_summary(
                        call["name"].as_str().or(call["title"].as_str()).unwrap_or("tool"),
                        &call["rawInput"],
                    ))];
                }
            }
            Some("usage_update") => {
                if let Some(used) = update["used"].as_u64() {
                    self.usage.context_tokens = Some(used);
                }
            }
            _ => {}
        }
        vec![]
    }

    fn permission(&mut self, ev: &Value) -> Vec<Event> {
        let params = &ev["params"];
        if params["sessionId"].as_str() != self.session.as_deref() {
            self.outbound.push(cancelled(&ev["id"]));
            return vec![];
        }
        let mut call = params["toolCall"]["toolCallId"]
            .as_str()
            .and_then(|id| self.calls.get(id))
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(fields) = params["toolCall"].as_object() {
            for (k, v) in fields {
                call[k] = v.clone();
            }
        }
        let id = ev["id"].to_string();
        self.permissions.insert(id.clone(), ev.clone());
        let raw = &call["rawInput"];
        // Only known raw command inputs get Bash rules/auto approval. Never infer
        // an executable command from the model-authored title or display content.
        let (tool, input) = if call["kind"] == "execute" && raw["command"].is_string() {
            ("Bash".to_string(), raw.clone())
        } else {
            (call["name"].as_str().unwrap_or("AntigravityTool").to_string(), call.clone())
        };
        let summary = crate::permissions::describe(&tool, &input);
        vec![Event::PermissionRequest(PermissionRequest { id, tool, input, summary, blocked_path: None })]
    }

    pub fn answer(&mut self, req: &PermissionRequest, decision: &Decision) -> Option<String> {
        let ev = self.permissions.remove(&req.id)?;
        let wanted = match decision {
            Decision::Allow | Decision::AllowAlways => "allow_once",
            Decision::Deny { .. } => "reject_once",
        };
        // The host owns remembered rules; never let an ACP "always" option
        // persist a broader permission inside Antigravity.
        let option = ev["params"]["options"]
            .as_array()
            .and_then(|options| options.iter().find(|o| o["kind"] == wanted))
            .and_then(|o| o["optionId"].as_str());
        Some(match option {
            Some(option) => format!(
                "{}\n",
                json!({"jsonrpc":"2.0","id":ev["id"],
                "result":{"outcome":{"outcome":"selected","optionId":option}}})
            ),
            None => cancelled(&ev["id"]),
        })
    }
}

fn cancelled(id: &Value) -> String {
    format!("{}\n", json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::gemini::{Flavor, GeminiBackend};
    use crate::backend::{self, Approver, Backend};

    fn request(mode: Mode, session: Option<&str>) -> TurnRequest {
        TurnRequest {
            dir: "/tmp".into(),
            model: "gemini-3.8-flash".into(),
            prompt: "explain".into(),
            images: vec![],
            session_id: session.map(String::from),
            mode,
        }
    }

    fn setup(acp: &mut Acp, mode: Mode, session: Option<&str>) {
        acp.start(&request(mode, session));
        assert!(acp.parse(&json!({"id":1,"result":{"protocolVersion":1}})).is_empty());
        let sent: Value = serde_json::from_str(&acp.drain()[0]).unwrap();
        assert_eq!(sent["method"], if session.is_some() { "session/load" } else { "session/new" });
        assert!(acp
            .parse(&json!({"id":2,"result":{"sessionId":"s","modes":{"availableModes":[
            {"id":"default"},{"id":"accept-edits"},{"id":"plan"}]}}}))
            .is_empty());
        let sent: Value = serde_json::from_str(&acp.drain()[0]).unwrap();
        assert_eq!(sent["params"]["modelId"], "gemini-3.8-flash");
        acp.parse(&json!({"id":3,"result":{}}));
        let sent: Value = serde_json::from_str(&acp.drain()[0]).unwrap();
        assert_eq!(
            sent["params"]["modeId"],
            match mode {
                Mode::Ask => "default",
                Mode::Plan => "plan",
                _ => "accept-edits",
            }
        );
        acp.parse(&json!({"id":4,"result":{}}));
        let sent: Value = serde_json::from_str(&acp.drain()[0]).unwrap();
        assert_eq!(sent["method"], "session/prompt");
        assert_eq!(sent["params"]["prompt"][0]["text"], "explain");
    }

    fn update(update: Value) -> Value {
        json!({"method":"session/update","params":{"sessionId":"s","update":update}})
    }

    fn permission() -> Value {
        json!({"jsonrpc":"2.0","id":"permission-1","method":"session/request_permission","params":{
            "sessionId":"s", "toolCall":{"toolCallId":"c"}, "options":[
                {"optionId":"yes","kind":"allow_once"}, {"optionId":"no","kind":"reject_once"},
                {"optionId":"forever","kind":"allow_always"}]}})
    }

    #[test]
    fn all_modes_and_resumed_sessions_complete_setup() {
        for mode in Mode::ALL {
            for session in [None, Some("s")] {
                setup(&mut Acp::default(), mode, session);
            }
        }
    }

    #[test]
    fn tool_only_start_asks_host_then_finishes_with_real_answer() {
        let mut acp = Acp::default();
        setup(&mut acp, Mode::Auto, None);
        let events = acp.parse(&update(json!({"sessionUpdate":"tool_call","toolCallId":"c",
            "name":"run_command","kind":"execute","rawInput":{"command":"git status --short"}})));
        assert!(matches!(&events[..], [Event::Tool(t)] if t.contains("git status")));
        let events = acp.parse(&permission());
        let [Event::PermissionRequest(req)] = &events[..] else { panic!("{events:?}") };
        assert_eq!(req.tool, "Bash");
        assert_eq!(req.input, json!({"command":"git status --short"}));
        let reply: Value = serde_json::from_str(&acp.answer(req, &Decision::AllowAlways).unwrap()).unwrap();
        assert_eq!(reply["id"], "permission-1");
        assert_eq!(reply["result"]["outcome"]["optionId"], "yes");
        acp.parse(&update(
            json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"private reasoning"}}),
        ));
        for text in ["Working tree ", "is clean."] {
            acp.parse(&update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}})));
        }
        let events = acp.parse(&json!({"id":5,"result":{"stopReason":"end_turn"}}));
        assert!(
            matches!(&events[..], [Event::Finished(f)] if f.text == "Working tree is clean." && f.session_id.as_deref() == Some("s"))
        );
    }

    #[test]
    fn denial_and_missing_once_option_never_grant_permission() {
        let mut acp = Acp::default();
        setup(&mut acp, Mode::Edit, None);
        for missing in [false, true] {
            let mut ev = permission();
            if missing {
                ev["params"]["options"] = json!([{"optionId":"forever","kind":"allow_always"}]);
            }
            let events = acp.parse(&ev);
            let [Event::PermissionRequest(req)] = &events[..] else { panic!() };
            let reply: Value =
                serde_json::from_str(&acp.answer(req, &Decision::Deny { reason: "no".into() }).unwrap()).unwrap();
            if missing {
                assert_eq!(reply["result"]["outcome"]["outcome"], "cancelled");
            } else {
                assert_eq!(reply["result"]["outcome"]["optionId"], "no");
            }
        }
    }

    #[test]
    fn titles_cannot_turn_unknown_tools_into_auto_approved_commands() {
        let mut acp = Acp::default();
        setup(&mut acp, Mode::Auto, None);
        let mut ev = permission();
        ev["params"]["toolCall"] = json!({"toolCallId":"x","title":"git status","kind":"execute"});
        let events = acp.parse(&ev);
        let [Event::PermissionRequest(req)] = &events[..] else { panic!() };
        assert_ne!(req.tool, "Bash");
    }

    #[test]
    fn errors_cancellation_and_blank_answers_remain_failures() {
        let mut acp = Acp::default();
        setup(&mut acp, Mode::Edit, None);
        for ev in [
            json!({"id":3,"error":{"message":"unknown model"}}),
            json!({"id":5,"result":{"stopReason":"cancelled"}}),
            json!({"id":5,"result":{"stopReason":"end_turn"}}),
        ] {
            assert!(matches!(&acp.parse(&ev)[..], [Event::Failed { .. }]));
        }
    }

    #[test]
    fn replayed_history_is_not_the_new_answer() {
        let mut acp = Acp::default();
        acp.start(&request(Mode::Edit, Some("s")));
        acp.parse(&json!({"id":1,"result":{"protocolVersion":1}}));
        assert!(acp
            .parse(&update(
                json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old answer"}})
            ))
            .is_empty());
        assert!(acp.text.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn actual_runner_exchanges_setup_and_tool_approval_over_pipes() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("jev-acp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-agy");
        std::fs::write(&script, r##"#!/usr/bin/python3
import sys,json
assert sys.argv[1:] == ['acp']
def read(): return json.loads(sys.stdin.readline())
def send(obj): print(json.dumps(obj), flush=True)
assert read()['method'] == 'initialize'
send({'id':1,'result':{'protocolVersion':1}})
assert read()['method'] == 'session/new'
send({'id':2,'result':{'sessionId':'s','modes':{'availableModes':[{'id':'accept-edits'}]}}})
assert read()['params']['modelId'] == 'gemini-3.8-flash'
send({'id':3,'result':{}})
assert read()['params']['modeId'] == 'accept-edits'
send({'id':4,'result':{}})
assert read()['method'] == 'session/prompt'
send({'id':99,'method':'session/request_permission','params':{'sessionId':'s','toolCall':{'toolCallId':'c','kind':'execute','name':'run_command','rawInput':{'command':'git status --short'}},'options':[{'kind':'allow_once','optionId':'ok'},{'kind':'reject_once','optionId':'no'}]}})
decision = read()['result']['outcome']['optionId']
assert decision in ('ok', 'no')
send({'method':'session/update','params':{'sessionId':'s','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'Clean.' if decision == 'ok' else 'Command denied.'}}}})
send({'id':5,'result':{'stopReason':'end_turn'}})
assert sys.stdin.read() == ''
"##).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        struct Allow;
        impl Approver for Allow {
            fn decide<'a>(
                &'a self,
                req: &'a PermissionRequest,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Decision> + Send + 'a>> {
                assert_eq!(req.tool, "Bash");
                assert_eq!(req.summary, "git status --short");
                Box::pin(async { Decision::Allow })
            }
        }
        let backend = GeminiBackend::new(Flavor::Antigravity, script.to_string_lossy());
        assert!(backend.interactive());
        let outcome =
            backend::run_turn_with(&backend, &request(Mode::Auto, None), Duration::from_secs(5), &mut |_| {}, &Allow)
                .await;
        let denied = backend::run_turn_with(
            &backend,
            &request(Mode::Auto, None),
            Duration::from_secs(5),
            &mut |_| {},
            &backend::NoApprover,
        )
        .await;
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(denied.unwrap().text, "Command denied.");
        let outcome = outcome.unwrap();
        assert_eq!(outcome.text, "Clean.");
        assert_eq!(outcome.session_id.as_deref(), Some("s"));
    }
}
