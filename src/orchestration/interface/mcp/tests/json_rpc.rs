//! Protocol-level tests: handshake, notifications, ping, unknown methods and bad JSON.

use serde_json::{json, Value};

use super::{rig, send, call};
use crate::orchestration::interface::mcp::json_rpc::{handle, PROTOCOL_VERSIONS};

#[tokio::test]
async fn initialize_answers_version_capabilities_and_server_info() {
    let (s, _) = rig();
    let r = send(&s, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "x", "version": "1" } } })).await;
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], "2025-03-26", "a version we speak is echoed");
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert_eq!(r["result"]["serverInfo"]["name"], "jev-delegate");
    let r = send(&s, json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": { "protocolVersion": "1999-01-01" } })).await;
    assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[0], "an unknown version gets our newest");
}

#[tokio::test]
async fn notifications_and_stray_responses_get_no_reply() {
    let (s, _) = rig();
    assert!(handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &s).await.is_none());
    assert!(handle(r#"{"jsonrpc":"2.0","method":"unknown/thing"}"#, &s).await.is_none(), "even an unknown notification");
    assert!(handle(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#, &s).await.is_none(), "a response to nothing we sent");
}

#[tokio::test]
async fn ping_is_answered() {
    let (s, _) = rig();
    let r = send(&s, json!({ "jsonrpc": "2.0", "id": "p", "method": "ping" })).await;
    assert_eq!((r["id"].clone(), r["result"].clone()), (json!("p"), json!({})));
}

#[tokio::test]
async fn unknown_methods_and_tools_are_json_rpc_errors() {
    let (s, _) = rig();
    let r = send(&s, json!({ "jsonrpc": "2.0", "id": 11, "method": "resources/list" })).await;
    assert_eq!((r["id"].clone(), r["error"]["code"].clone()), (json!(11), json!(-32601)));
    let r = send(&s, call(12, "spawn_anything", json!({}))).await;
    assert_eq!(r["error"]["code"], -32602);
    assert!(r["error"]["message"].as_str().unwrap().contains("unknown tool"));
}

#[tokio::test]
async fn malformed_json_is_a_parse_error_with_a_null_id() {
    let (s, _) = rig();
    for bad in ["{not json", "", "[1,2]", "42"] {
        let r: Value = serde_json::from_str(&handle(bad, &s).await.unwrap()).unwrap();
        assert!(r["id"].is_null(), "{bad}");
        assert!([-32700, -32600].contains(&r["error"]["code"].as_i64().unwrap()), "{bad}");
    }
}
