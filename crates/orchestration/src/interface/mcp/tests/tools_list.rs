//! The advertised tool list: names, annotations, schemas and descriptions.

use serde_json::json;

use super::{rig, send};

#[tokio::test]
async fn tools_list_has_six_tools_and_delegate_has_no_model_field() {
    let (s, _) = rig();
    let r = send(&s, json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" })).await;
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["delegate", "wait", "show_diff", "merge", "discard", "undo"]);
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object");
        assert!(t["inputSchema"]["required"].is_array());
    }
    // Starting a task changes nothing in the caller's tree; collecting it may merge.
    assert_eq!(tools[0]["annotations"], json!({ "readOnlyHint": true, "destructiveHint": false, "openWorldHint": true }));
    assert_eq!(tools[1]["annotations"], json!({ "readOnlyHint": false, "destructiveHint": false }));
    assert_eq!(tools[2]["annotations"], json!({ "readOnlyHint": true }));
    assert_eq!(tools[3]["annotations"], json!({ "readOnlyHint": false, "destructiveHint": false }));
    assert_eq!(tools[4]["annotations"], json!({ "readOnlyHint": false, "destructiveHint": true }));
    assert_eq!(tools[5]["annotations"], json!({ "readOnlyHint": false, "destructiveHint": true }), "undo changes the working tree");
    assert_eq!(tools[5]["inputSchema"]["required"], json!(["worker_id"]));
    let wait = &tools[1];
    assert_eq!(wait["inputSchema"]["required"], json!([]), "everything is optional");
    assert_eq!(wait["inputSchema"]["properties"]["timeout_seconds"]["type"], "integer");
    let w = wait["description"].as_str().unwrap();
    assert!(w.contains("every task not yet collected") && w.contains("auto-merge") && w.contains("never while you work"), "{w}");
    let delegate = &tools[0];
    let props = delegate["inputSchema"]["properties"].as_object().unwrap();
    assert_eq!(props.keys().map(String::as_str).collect::<Vec<_>>(), ["brief", "files", "read_only", "review"]);
    assert_eq!(props["review"]["type"], "boolean");
    assert!(!props.contains_key("model") && !delegate.to_string().contains("\"model\""));
    assert_eq!(delegate["inputSchema"]["required"], json!(["brief"]));
    // The descriptions carry the guidance the planner needs.
    let d = delegate["description"].as_str().unwrap();
    assert!(d.contains("independent") && d.contains("self-contained") && d.contains("review the diff yourself") && d.contains("merge only what you verified"));
    assert!(d.contains("EVERY file") && d.contains("show_diff"));
    assert!(d.contains("AUTO-MERGED") && d.contains("undo(worker_id)") && d.contains("plan mode") && d.contains("uncommitted edits"));
    assert!(d.contains("task id AT ONCE") && d.contains("background") && d.contains("call wait"));
    let show = &tools[2];
    assert_eq!(show["inputSchema"]["required"], json!(["worker_id"]));
    let props = show["inputSchema"]["properties"].as_object().unwrap();
    assert_eq!(props.keys().map(String::as_str).collect::<Vec<_>>(), ["file", "offset", "worker_id"]);
    assert_eq!(props["offset"]["type"], "integer");
    assert!(show["description"].as_str().unwrap().contains("pages"));
    let merge = &tools[3];
    assert_eq!(merge["inputSchema"]["required"], json!(["worker_id"]), "files is optional");
    assert_eq!(merge["inputSchema"]["properties"]["files"]["type"], "array");
    let m = merge["description"].as_str().unwrap();
    assert!(m.contains("after you reviewed") && m.contains("REFUSES") && m.contains("show_diff") && m.contains("files"));
    assert!(!tools.iter().any(|t| t.to_string().contains("\"model\"")), "no tool takes a model");
}
