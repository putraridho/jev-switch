//! Tests of the Claude backend, by subject; the fakes and recorded fixtures are shared here.

use serde_json::{json, Value};
use tokio::process::Command;

use super::*;
use crate::backend::{Decision, ErrorKind, Event, McpAttachment, PermissionRequest, ProviderId, TurnRequest};

mod command;
mod mcp_config;
mod permission_wire;
mod stream_events;
mod stream_input;


const TURN1: &str = fixture!("claude/turn1.jsonl");

fn parse_all(backend: &ClaudeBackend) -> Vec<Event> {
    TURN1.lines().flat_map(|l| backend.parse_line(l)).collect()
}

fn args(cmd: &Command) -> Vec<String> {
    cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect()
}

fn backend() -> ClaudeBackend {
    ClaudeBackend {
        bin: "claude".into(),
        provider: ProviderId::Claude,
        env: vec![],
        allowed_tools: None,
        mcp_config: None,
        mcp_name: None,
        interactive: false,
        streaming: false,
    }
}

fn req(session: Option<&str>) -> TurnRequest {
    TurnRequest {
        dir: "/tmp".into(),
        model: "sonnet-x".into(),
        prompt: "hi".into(),
        images: vec![],
        session_id: session.map(String::from),
        mode: Default::default(),
        trust_project_settings: false,
    }
}

fn attachment() -> McpAttachment {
    McpAttachment {
        name: "jev".into(),
        command: "/bin/delegate_mcp".into(),
        args: vec!["--dir".into(), "/repo".into()],
        config_path: "/state/mcp.json".into(),
    }
}

const ALLOW_TURN: &str = fixture!("claude/permission_allow.jsonl");
const DENY_TURN: &str = fixture!("claude/permission_deny.jsonl");
const ALLOW_LINE: &str = fixture!("claude/control_response_allow.jsonl");
const DENY_LINE: &str = fixture!("claude/control_response_deny.jsonl");
const USER_LINE: &str = fixture!("claude/user_message.jsonl");

fn session() -> ClaudeBackend {
    backend().in_session()
}

fn request_from(turn: &str) -> PermissionRequest {
    let events: Vec<Event> = turn.lines().flat_map(|l| backend().parse_line(l)).collect();
    events
        .into_iter()
        .find_map(|e| match e {
            Event::PermissionRequest(r) => Some(r),
            _ => None,
        })
        .expect("the recorded turn holds a control request")
}

fn json_of(line: &str) -> Value {
    serde_json::from_str(line).unwrap()
}

/// One golden case: `=== name`, then the arguments as a JSON array on one line. The
/// `--setting-sources` value depends on the developer's trust settings, so it is masked.
fn golden_case(name: &str, cmd: &Command) -> String {
    let mut a = args(cmd);
    if let Some(i) = a.iter().position(|x| x == "--setting-sources") {
        a[i + 1] = "<SOURCES>".into();
    }
    format!("=== {name}\n{}\n", serde_json::to_string(&a).unwrap())
}

fn spec(p: ProviderId, id: &str) -> &'static crate::catalog::domain::model_table::ModelSpec {
    crate::catalog::domain::model_table::find(p, id).unwrap()
}
