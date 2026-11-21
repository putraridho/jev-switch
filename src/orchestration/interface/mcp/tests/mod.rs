//! Tests of the MCP server, with a fake delegation API that records every call.

mod compose;
mod json_rpc;
mod tool_calls;
mod tools_list;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::json_rpc::{handle, Server};
use crate::orchestration::application::ports::{BoxFut, DelegateApi};
use crate::orchestration::domain::delegation::{DelegateRequest, DelegateResult, Landing, ReviewReason};
use crate::orchestration::domain::file_patch::FileStat;

/// The arguments of one `wait` call: task ids and timeout.
type WaitCall = (Option<Vec<String>>, Option<Duration>);

/// Records every call; its worker report is canned.
#[derive(Default)]
pub(super) struct FakeApi {
    pub(super) delegated: Mutex<Vec<DelegateRequest>>,
    pub(super) waited: Mutex<Vec<WaitCall>>,
    pub(super) merged: Mutex<Vec<(String, Option<Vec<String>>)>>,
    pub(super) shown: Mutex<Vec<(String, Option<String>, usize)>>,
    pub(super) discarded: Mutex<Vec<String>>,
    pub(super) undone: Mutex<Vec<String>>,
}

/// The canned result of task t1: a worker waiting for review.
fn canned_result() -> DelegateResult {
    DelegateResult {
        worker_id: "w1".into(),
        provider: Some(crate::backend::ProviderId::Codex),
        model: Some("gpt-6.1-terra".into()),
        summary: "added the retry".into(),
        stats: vec![
            FileStat { path: "src/net.rs".into(), added: 3, removed: 1, binary: false },
            FileStat { path: "src/big.rs".into(), added: 900, removed: 0, binary: false },
        ],
        diff: "diff --git a/src/net.rs b/src/net.rs".into(),
        shown: vec!["src/net.rs".into()],
        ok: true,
        error: None,
        retries: vec![],
        landing: Landing::Review(ReviewReason::NoTestCommand),
    }
}

impl DelegateApi for FakeApi {
    fn start(&self, req: DelegateRequest) -> BoxFut<'_, Result<String, String>> {
        let unavailable = req.brief == "unavailable";
        self.delegated.lock().unwrap().push(req);
        Box::pin(async move { if unavailable { Err("delegation is not available: no Jev key".into()) } else { Ok("t1".into()) } })
    }
    fn wait(&self, ids: Option<Vec<String>>, timeout: Option<Duration>) -> BoxFut<'_, Result<String, String>> {
        self.waited.lock().unwrap().push((ids, timeout));
        Box::pin(async { Ok(format!("task t1: {}", canned_result().render())) })
    }
    fn show_diff(&self, id: String, file: Option<String>, offset: usize) -> BoxFut<'_, Result<String, String>> {
        self.shown.lock().unwrap().push((id.clone(), file, offset));
        Box::pin(async move { Ok(format!("diff page of {id}")) })
    }
    fn merge(&self, id: String, files: Option<Vec<String>>) -> BoxFut<'_, Result<String, String>> {
        self.merged.lock().unwrap().push((id.clone(), files));
        Box::pin(async move { if id == "conflict" { Err("does not apply cleanly".into()) } else { Ok(format!("merged {id}")) } })
    }
    fn discard(&self, id: String) -> BoxFut<'_, Result<String, String>> {
        self.discarded.lock().unwrap().push(id.clone());
        Box::pin(async move { Ok(format!("discarded {id}")) })
    }
    fn undo(&self, id: String) -> BoxFut<'_, Result<String, String>> {
        self.undone.lock().unwrap().push(id.clone());
        Box::pin(async move { if id == "moved-on" { Err("could not undo".into()) } else { Ok(format!("undid {id}")) } })
    }
}

pub(super) fn rig() -> (Server, Arc<FakeApi>) {
    let api = Arc::new(FakeApi::default());
    (Server { api: api.clone() }, api)
}

pub(super) async fn send(server: &Server, msg: Value) -> Value {
    let reply = handle(&msg.to_string(), server).await.expect("a reply");
    serde_json::from_str(&reply).unwrap()
}

pub(super) fn call(id: u64, name: &str, args: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": args } })
}
