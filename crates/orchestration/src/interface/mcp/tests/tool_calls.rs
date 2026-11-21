//! tools/call: arguments reach the delegation API, bad ones are invalid params.

use std::time::Duration;

use serde_json::json;

use super::{call, rig, send};
use crate::domain::delegation::DelegateRequest;
use crate::interface::mcp::tool_arguments::parse_delegate;

#[tokio::test]
async fn delegate_starts_a_task_and_wait_returns_its_report() {
    let (s, api) = rig();
    let r = send(&s, call(4, "delegate", json!({ "brief": "add a retry", "files": ["src/net.rs"], "read_only": false, "review": true }))).await;
    assert_eq!(r["result"]["isError"], false);
    let started = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(started.starts_with("started task t1") && started.contains("call wait") && started.contains("Nothing lands"), "{started}");
    assert_eq!(*api.delegated.lock().unwrap(), vec![DelegateRequest { brief: "add a retry".into(), files: vec!["src/net.rs".into()], read_only: false, review: true }]);
    let r = send(&s, call(40, "wait", json!({}))).await;
    assert_eq!(r["result"]["isError"], false);
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("task t1: worker w1") && text.contains("added the retry") && text.contains("diff"), "{text}");
    assert!(text.contains("merge(w1"));
    // Every changed file is listed with its counts, and the one not shown says how to read it.
    assert!(text.contains("src/net.rs +3 -1") && text.contains("src/big.rs +900 -0"), "{text}");
    assert!(text.contains("NOT SHOWN") && text.contains("src/big.rs") && text.contains("show_diff(w1, file)"), "{text}");
    assert!(text.contains("Not auto-merged"), "{text}");
}

#[tokio::test]
async fn wait_passes_task_ids_and_timeout_and_rejects_bad_ones() {
    let (s, api) = rig();
    send(&s, call(41, "wait", json!({ "task_ids": ["t1", "t2"], "timeout_seconds": 30 }))).await;
    send(&s, call(42, "wait", json!({ "timeout_seconds": 0 }))).await;
    assert_eq!(
        *api.waited.lock().unwrap(),
        [(Some(vec!["t1".to_string(), "t2".to_string()]), Some(Duration::from_secs(30))), (None, Some(Duration::ZERO))]
    );
    for args in [json!({ "task_ids": "t1" }), json!({ "task_ids": [1] }), json!({ "timeout_seconds": -1 }), json!({ "timeout_seconds": "5" })] {
        assert_eq!(send(&s, call(43, "wait", args.clone())).await["error"]["code"], -32602, "{args}");
    }
    assert_eq!(api.waited.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_delegation_that_cannot_start_is_an_error_the_model_sees() {
    let (s, _) = rig();
    let r = send(&s, call(44, "delegate", json!({ "brief": "unavailable" }))).await;
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("not available"));
}

#[tokio::test]
async fn a_model_argument_is_ignored() {
    let (s, api) = rig();
    // Nor can show_diff or merge be steered by a model: they read only their own keys.
    send(&s, call(50, "show_diff", json!({ "worker_id": "w1", "model": "claude-opus-5-5" }))).await;
    send(&s, call(51, "merge", json!({ "worker_id": "w1", "model": "claude-opus-5-5", "provider": "codex" }))).await;
    assert_eq!(*api.shown.lock().unwrap(), [("w1".to_string(), None, 0)]);
    assert_eq!(*api.merged.lock().unwrap(), [("w1".to_string(), None)]);
    api.shown.lock().unwrap().clear();
    api.merged.lock().unwrap().clear();
    let r = send(&s, call(5, "delegate", json!({ "brief": "do it", "model": "claude-opus-5-5", "provider": "codex", "read_only": true }))).await;
    assert_eq!(r["result"]["isError"], false, "not rejected, just not heard");
    // The request the delegator received has only the three real fields; there is no place for a model.
    assert_eq!(*api.delegated.lock().unwrap(), vec![DelegateRequest { brief: "do it".into(), files: vec![], read_only: true, review: false }]);
    assert_eq!(parse_delegate(&json!({ "brief": "x", "model": "opus" })).unwrap(), parse_delegate(&json!({ "brief": "x" })).unwrap());
}

#[tokio::test]
async fn bad_delegate_arguments_are_invalid_params() {
    let (s, api) = rig();
    for args in [json!({}), json!({ "brief": "   " }), json!({ "brief": 5 }), json!({ "brief": "x", "files": "a.rs" }), json!({ "brief": "x", "files": [1] })] {
        let r = send(&s, call(6, "delegate", args.clone())).await;
        assert_eq!(r["error"]["code"], -32602, "{args}");
    }
    assert!(api.delegated.lock().unwrap().is_empty());
}

#[tokio::test]
async fn merge_and_discard_pass_the_worker_id_through() {
    let (s, api) = rig();
    let r = send(&s, call(7, "merge", json!({ "worker_id": "w1" }))).await;
    assert_eq!((r["result"]["isError"].clone(), r["result"]["content"][0]["text"].clone()), (json!(false), json!("merged w1")));
    let r = send(&s, call(8, "merge", json!({ "worker_id": "conflict" }))).await;
    assert_eq!(r["result"]["isError"], true, "a failed merge is visible to the model, not a protocol error");
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("does not apply cleanly"));
    let r = send(&s, call(9, "discard", json!({ "worker_id": "w2" }))).await;
    assert_eq!(r["result"]["content"][0]["text"], "discarded w2");
    assert_eq!(*api.merged.lock().unwrap(), [("w1".to_string(), None), ("conflict".to_string(), None)]);
    assert_eq!(*api.discarded.lock().unwrap(), ["w2"]);
    let r = send(&s, call(10, "merge", json!({}))).await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn undo_passes_the_worker_id_through_and_shows_a_refusal_to_the_model() {
    let (s, api) = rig();
    let r = send(&s, call(30, "undo", json!({ "worker_id": "w1", "model": "x" }))).await;
    assert_eq!((r["result"]["isError"].clone(), r["result"]["content"][0]["text"].clone()), (json!(false), json!("undid w1")));
    let r = send(&s, call(31, "undo", json!({ "worker_id": "moved-on" }))).await;
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(send(&s, call(32, "undo", json!({}))).await["error"]["code"], -32602);
    assert_eq!(*api.undone.lock().unwrap(), ["w1", "moved-on"]);
}

#[tokio::test]
async fn show_diff_passes_file_and_offset_and_merge_passes_files() {
    let (s, api) = rig();
    let r = send(&s, call(20, "show_diff", json!({ "worker_id": "w1", "file": "src/big.rs", "offset": 20000 }))).await;
    assert_eq!((r["result"]["isError"].clone(), r["result"]["content"][0]["text"].clone()), (json!(false), json!("diff page of w1")));
    send(&s, call(21, "show_diff", json!({ "worker_id": "w1" }))).await;
    assert_eq!(*api.shown.lock().unwrap(), [("w1".to_string(), Some("src/big.rs".to_string()), 20000), ("w1".to_string(), None, 0)]);
    send(&s, call(22, "merge", json!({ "worker_id": "w1", "files": ["src/net.rs"] }))).await;
    assert_eq!(*api.merged.lock().unwrap(), [("w1".to_string(), Some(vec!["src/net.rs".to_string()]))]);
}

#[tokio::test]
async fn bad_show_diff_and_merge_arguments_are_invalid_params() {
    let (s, api) = rig();
    for (tool, args) in [
        ("show_diff", json!({})),
        ("show_diff", json!({ "worker_id": "w1", "offset": -1 })),
        ("show_diff", json!({ "worker_id": "w1", "offset": "0" })),
        ("show_diff", json!({ "worker_id": "w1", "file": 5 })),
        ("show_diff", json!({ "worker_id": "w1", "file": "" })),
        ("merge", json!({ "worker_id": "w1", "files": "a.rs" })),
        ("merge", json!({ "worker_id": "w1", "files": [1] })),
    ] {
        let r = send(&s, call(23, tool, args.clone())).await;
        assert_eq!(r["error"]["code"], -32602, "{tool} {args}");
    }
    assert!(api.shown.lock().unwrap().is_empty() && api.merged.lock().unwrap().is_empty());
}
