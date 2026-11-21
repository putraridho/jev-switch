//! Tests of the Antigravity ACP transport, by subject; the request and session helpers are shared here.

use serde_json::{json, Value};

use super::*;
use jev_approval::domain::mode::Mode;
use crate::{Event, TurnRequest};

mod permission;
mod runner;
mod session;
mod tool_rows;

pub(super) fn request(mode: Mode, session: Option<&str>) -> TurnRequest {
    TurnRequest {
        dir: "/tmp".into(),
        model: "gemini-3.8-flash".into(),
        prompt: "explain".into(),
        images: vec![],
        session_id: session.map(String::from),
        mode,
        trust_project_settings: false,
    }
}

pub(super) fn setup(acp: &mut Acp, mode: Mode, session: Option<&str>) {
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

pub(super) fn update(update: Value) -> Value {
    json!({"method":"session/update","params":{"sessionId":"s","update":update}})
}

pub(super) fn permission() -> Value {
    json!({"jsonrpc":"2.0","id":"permission-1","method":"session/request_permission","params":{
        "sessionId":"s", "toolCall":{"toolCallId":"c"}, "options":[
            {"optionId":"yes","kind":"allow_once"}, {"optionId":"no","kind":"reject_once"},
            {"optionId":"forever","kind":"allow_always"}]}})
}
