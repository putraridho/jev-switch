//! JSON-RPC 2.0 framing: one line in, at most one line out.

use std::sync::Arc;

use serde_json::{json, Value};

use super::tool_arguments::call_tool;
use super::tool_schema::tools_list;
use crate::application::ports::DelegateApi;

/// Protocol versions this server speaks, newest first. A client's requested version is
/// echoed when it is one of these, else the newest is offered (the client then decides).
pub(super) const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

fn rpc_result(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn rpc_error(id: &Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

fn initialize_result(params: &Value) -> Value {
    let requested = params["protocolVersion"].as_str();
    let version = PROTOCOL_VERSIONS.iter().find(|v| Some(**v) == requested).unwrap_or(&PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "jev-delegate", "version": env!("CARGO_PKG_VERSION") }
    })
}

/// What the protocol layer needs: the delegation API. Cloning shares it, which is how
/// parallel `delegate` calls run concurrently.
#[derive(Clone)]
pub(super) struct Server {
    pub(super) api: Arc<dyn DelegateApi>,
}

/// One line in, at most one line out. A notification (no `id`) never gets a reply.
pub(super) async fn handle(line: &str, server: &Server) -> Option<String> {
    let Ok(msg) = serde_json::from_str::<Value>(line) else {
        return Some(rpc_error(&Value::Null, -32700, "parse error: not valid JSON"));
    };
    if !msg.is_object() {
        return Some(rpc_error(&Value::Null, -32600, "invalid request: expected a JSON object (batches are not supported)"));
    }
    let id = msg.get("id").filter(|i| !i.is_null())?.clone();
    let Some(method) = msg["method"].as_str() else {
        // A response to something we never sent, or junk with an id.
        return msg.get("result").is_none().then(|| rpc_error(&id, -32600, "invalid request: no method"));
    };
    let params = &msg["params"];
    Some(match method {
        "initialize" => rpc_result(&id, initialize_result(params)),
        "ping" => rpc_result(&id, json!({})),
        "tools/list" => rpc_result(&id, tools_list()),
        "tools/call" => match call_tool(server, params).await {
            Ok(result) => rpc_result(&id, result),
            Err(message) => rpc_error(&id, -32602, &message),
        },
        other => rpc_error(&id, -32601, &format!("method not found: {other}")),
    })
}
