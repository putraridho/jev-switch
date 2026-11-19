//! delegate_mcp: the MCP server that gives an orchestration PLANNER its `delegate`,
//! `show_diff`, `merge` and `discard` tools. A planner CLI (Claude Code, Codex) starts it as a stdio
//! child: newline-delimited JSON-RPC 2.0, one message per line.
//!
//! The tool has NO model parameter. Which worker model runs a piece is Jev's decision
//! (see `jev_router::delegate`); anything the planner says about models is ignored.
//!
//!   delegate_mcp [--dir <project dir>] [--planner <provider:model>]
//!
//! Provider probing and Jev are set up lazily on the first `delegate` call, so the
//! handshake is instant. Stdout is the protocol channel: this binary prints nothing else
//! there. `handle` is the whole protocol as a pure-ish function so it is unit tested
//! without a process.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, OnceCell};
use tokio::task::JoinSet;

use jev_router::delegate::{has_leftovers, router_judge, BoxFut, DelegateApi, DelegateRequest, DelegateResult, Delegator, GitWorktrees};
use jev_router::log;
use jev_router::providers::Runtime;
use jev_router::router::Router;
use jev_router::typesafe::TypeSafe;

/// Protocol versions this server speaks, newest first. A client's requested version is
/// echoed when it is one of these, else the newest is offered (the client then decides).
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const DELEGATE_DESCRIPTION: &str = "Hand ONE self-contained piece of a larger task to a worker model, which runs in its own \
private git worktree and returns a short report, the list of EVERY file it changed (with +/- counts) and the diffs of as many \
files as fit. Use it for independent pieces (you may call it several times in parallel; each worker gets its own directory). \
Give each call a COMPLETE, self-contained brief: the worker sees only the brief and the last commit, not your conversation and \
not your uncommitted edits, so commit what a worker needs first or put it in the brief. There is no model parameter: the worker \
model is chosen for you. Review the returned diff yourself before merging; merge only what you verified. A file whose diff is \
not shown in the result must be read with show_diff before it can be merged.";
const SHOW_DIFF_DESCRIPTION: &str = "Read a worker's diff in pages (about 20,000 characters each). Without `file` it pages the \
whole diff; with `file` (a path from the delegate result's file list) it pages that one file, which is how to read a file the \
delegate result did not show. Pass `offset` (characters) to continue where the previous page ended. A file counts as seen once \
its diff was shown in full, and merge accepts only files that were seen.";
const MERGE_DESCRIPTION: &str = "Apply a worker's diff (by worker_id) to your working tree. It applies on top of your uncommitted \
edits when the hunks do not overlap, else as a 3-way merge. Do this only after you reviewed the diff and verified it: merge REFUSES \
any file you have not seen in full (call show_diff for it, or pass `files` listing only the files you have seen; the others stay in \
the worker's worktree for a later merge or discard). If it cannot apply, nothing is changed and the worker's worktree is kept so \
you can discard it or merge by hand.";
const DISCARD_DESCRIPTION: &str = "Throw away a worker's worktree and diff (by worker_id) without applying anything. Use it for \
work you rejected.";

fn worker_id_schema() -> Value {
    json!({ "type": "string", "description": "The worker_id a delegate call returned." })
}

fn tools_list() -> Value {
    json!({ "tools": [
        {
            "name": "delegate",
            "description": DELEGATE_DESCRIPTION,
            // Claude Code runs MCP calls concurrently only for tools marked readOnlyHint, and
            // serializes the rest. A worker changes only its own private git worktree, never the
            // caller's working tree, so it is safe to claim; this is what lets parallel delegations
            // fan out. Merging is a separate tool and is not read-only.
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "openWorldHint": true },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "brief": { "type": "string", "description": "The complete, self-contained task for the worker: goal, constraints, what done looks like. It sees nothing else." },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Optional paths the worker should look at first. Hints, not limits." },
                    "read_only": { "type": "boolean", "description": "True when the piece only reads and reports (no files change)." }
                },
                "required": ["brief"]
            }
        },
        {
            "name": "show_diff",
            "description": SHOW_DIFF_DESCRIPTION,
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "worker_id": worker_id_schema(),
                    "file": { "type": "string", "description": "A changed file's path: page just that file's diff." },
                    "offset": { "type": "integer", "minimum": 0, "description": "Character offset to continue from (default 0), as the previous page's footer says." }
                },
                "required": ["worker_id"]
            }
        },
        {
            "name": "merge",
            "description": MERGE_DESCRIPTION,
            "annotations": { "readOnlyHint": false, "destructiveHint": false },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "worker_id": worker_id_schema(),
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Optional: merge only these changed files (all of them must have been shown in full). Default: every file." }
                },
                "required": ["worker_id"]
            }
        },
        {
            "name": "discard",
            "description": DISCARD_DESCRIPTION,
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "properties": { "worker_id": worker_id_schema() },
                "required": ["worker_id"]
            }
        }
    ] })
}

/// The request the planner's `delegate` arguments mean. Reads `brief`, `files` and
/// `read_only` and NOTHING else: a `model` (or any other) key is dropped here, so no
/// model choice can pass through the tool.
fn parse_delegate(args: &Value) -> Result<DelegateRequest, String> {
    let brief = args["brief"].as_str().map(str::trim).filter(|b| !b.is_empty()).ok_or("`brief` is required: a non-empty string")?;
    let files = string_list(args, "files")?.unwrap_or_default();
    let read_only = args.get("read_only").and_then(Value::as_bool).unwrap_or(false);
    Ok(DelegateRequest { brief: brief.to_string(), files, read_only })
}

/// A `files` argument: absent (or null) is `None`, otherwise a list of strings.
fn string_list(args: &Value, key: &str) -> Result<Option<Vec<String>>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items.iter().map(|f| f.as_str().map(String::from).ok_or(format!("`{key}` must be a list of strings"))).collect::<Result<_, _>>().map(Some),
        Some(_) => Err(format!("`{key}` must be a list of strings")),
    }
}

/// `show_diff`'s optional `file` and `offset`.
fn parse_show_diff(args: &Value) -> Result<(Option<String>, usize), String> {
    let file = match args.get("file") {
        None | Some(Value::Null) => None,
        Some(Value::String(f)) if !f.is_empty() => Some(f.clone()),
        Some(_) => return Err("`file` must be a non-empty string".into()),
    };
    let offset = match args.get("offset") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().ok_or("`offset` must be a non-negative integer")? as usize,
    };
    Ok((file, offset))
}

fn worker_id_arg(args: &Value) -> Result<String, String> {
    args["worker_id"].as_str().filter(|w| !w.is_empty()).map(String::from).ok_or_else(|| "`worker_id` is required".to_string())
}

fn text_result(text: impl Into<String>, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": is_error })
}

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
struct Server {
    api: Arc<dyn DelegateApi>,
}

/// Run one tool. `Err` is a malformed call (a protocol error); a tool that ran and
/// failed is an `Ok` result flagged `isError`, which is how MCP lets the model see it.
async fn call_tool(server: &Server, params: &Value) -> Result<Value, String> {
    let name = params["name"].as_str().ok_or("tools/call needs a tool `name`")?;
    let args = &params["arguments"];
    match name {
        "delegate" => {
            let result = server.api.delegate(parse_delegate(args)?).await;
            Ok(text_result(result.render(), !result.ok))
        }
        "show_diff" => {
            let (file, offset) = parse_show_diff(args)?;
            Ok(outcome(server.api.show_diff(worker_id_arg(args)?, file, offset).await))
        }
        "merge" => {
            let files = string_list(args, "files")?;
            Ok(outcome(server.api.merge(worker_id_arg(args)?, files).await))
        }
        "discard" => Ok(outcome(server.api.discard(worker_id_arg(args)?).await)),
        other => Err(format!("unknown tool `{other}` (delegate, show_diff, merge, discard)")),
    }
}

fn outcome(r: Result<String, String>) -> Value {
    match r {
        Ok(text) => text_result(text, false),
        Err(text) => text_result(text, true),
    }
}

/// One line in, at most one line out. A notification (no `id`) never gets a reply.
async fn handle(line: &str, server: &Server) -> Option<String> {
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

// --------------------------------------------------------------- the real delegator

/// Builds the real [`Delegator`] on first use: probing providers takes seconds and the
/// MCP handshake must not wait for it.
struct Lazy {
    dir: PathBuf,
    planner: String,
    cell: OnceCell<Delegator>,
}

impl Lazy {
    async fn build(&self) -> Result<Delegator> {
        let router = Arc::new(Router::new(TypeSafe::from_env()?, 1.0));
        let state = log::state_dir().context("no state directory (is HOME set?)")?;
        let worktrees = GitWorktrees::new(&self.dir, state.join("worktrees"))?;
        let runtime = Runtime::start().await;
        let delegator = Delegator::new(runtime, router_judge(router), Box::new(worktrees))
            .with_log(log::path_from_env("JEV_DELEGATE_LOG", "orchestration.jsonl"))
            .with_planner(self.planner.clone())
            .with_guards(jev_router::guard::Guards::load(Some(&state), &self.dir))
            .with_store(&state);
        // Workers an earlier run left: reload the mergeable ones, remove the abandoned ones.
        delegator.recover().await;
        Ok(delegator)
    }

    async fn delegator(&self) -> Result<&Delegator> {
        self.cell.get_or_try_init(|| self.build()).await
    }

    /// The delegator for a tool that works on an existing worker: built on demand, because
    /// after a restart the worker lives only in the saved registry.
    async fn known(&self) -> Result<&Delegator, String> {
        self.delegator().await.map_err(|e| format!("delegation is not available: {e:#}"))
    }
}

impl DelegateApi for Lazy {
    fn delegate(&self, req: DelegateRequest) -> BoxFut<'_, DelegateResult> {
        Box::pin(async move {
            match self.delegator().await {
                Ok(d) => d.delegate(req).await,
                Err(e) => DelegateResult::failed(format!("delegation is not available: {e:#}. Do this piece yourself."), vec![]),
            }
        })
    }

    fn show_diff(&self, worker_id: String, file: Option<String>, offset: usize) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { self.known().await?.show_diff(&worker_id, file.as_deref(), offset).await })
    }

    fn merge(&self, worker_id: String, files: Option<Vec<String>>) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { self.known().await?.merge(&worker_id, files).await })
    }

    fn discard(&self, worker_id: String) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { self.known().await?.discard(&worker_id).await })
    }
}

/// `--dir` and `--planner` from the command line.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<(Option<PathBuf>, String)> {
    let (mut dir, mut planner) = (None, String::new());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dir" => dir = Some(PathBuf::from(it.next().context("--dir needs a value")?)),
            "--planner" => planner = it.next().context("--planner needs a value")?,
            other => anyhow::bail!("unknown option {other} (usage: delegate_mcp [--dir <dir>] [--planner <provider:model>])"),
        }
    }
    Ok((dir, planner))
}

#[tokio::main]
async fn main() -> Result<()> {
    let (dir, planner) = parse_args(std::env::args().skip(1))?;
    let dir = match dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };
    let lazy = Arc::new(Lazy { dir, planner, cell: OnceCell::new() });
    // Workers an earlier run left behind are cleaned up at startup, but providers are only
    // probed for that when there is something to clean: a session that never delegates stays free.
    if log::state_dir().is_some_and(|s| has_leftovers(&s)) {
        let warm = lazy.clone();
        tokio::spawn(async move {
            let _ = warm.delegator().await;
        });
    }
    let server = Server { api: lazy };

    // Replies go through one writer task so concurrent calls never interleave their lines.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut out = tokio::io::stdout();
        while let Some(line) = rx.recv().await {
            if out.write_all(line.as_bytes()).await.is_err() || out.write_all(b"\n").await.is_err() || out.flush().await.is_err() {
                break;
            }
        }
    });

    // One task per request, so a long `delegate` does not block the next one.
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut in_flight = JoinSet::new();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let (server, tx) = (server.clone(), tx.clone());
        in_flight.spawn(async move {
            if let Some(reply) = handle(&line, &server).await {
                let _ = tx.send(reply);
            }
        });
    }
    // stdin closed: answer what is still running, then stop. (A client that is really gone
    // also kills this process, and the worker CLIs die with it: they are killed on drop.)
    while in_flight.join_next().await.is_some() {}
    drop(tx);
    let _ = writer.await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use jev_router::delegate::FileStat;

    /// Records every call; its worker report is canned.
    #[derive(Default)]
    struct FakeApi {
        delegated: Mutex<Vec<DelegateRequest>>,
        merged: Mutex<Vec<(String, Option<Vec<String>>)>>,
        shown: Mutex<Vec<(String, Option<String>, usize)>>,
        discarded: Mutex<Vec<String>>,
    }

    impl DelegateApi for FakeApi {
        fn delegate(&self, req: DelegateRequest) -> BoxFut<'_, DelegateResult> {
            self.delegated.lock().unwrap().push(req);
            Box::pin(async {
                DelegateResult {
                    worker_id: "w1".into(),
                    provider: Some(jev_router::backend::ProviderId::Codex),
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
                }
            })
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
    }

    fn rig() -> (Server, Arc<FakeApi>) {
        let api = Arc::new(FakeApi::default());
        (Server { api: api.clone() }, api)
    }

    async fn send(server: &Server, msg: Value) -> Value {
        let reply = handle(&msg.to_string(), server).await.expect("a reply");
        serde_json::from_str(&reply).unwrap()
    }

    fn call(id: u64, name: &str, args: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": args } })
    }

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
    async fn tools_list_has_four_tools_and_delegate_has_no_model_field() {
        let (s, _) = rig();
        let r = send(&s, json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" })).await;
        let tools = r["result"]["tools"].as_array().unwrap();
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["delegate", "show_diff", "merge", "discard"]);
        for t in tools {
            assert_eq!(t["inputSchema"]["type"], "object");
            assert!(t["inputSchema"]["required"].is_array());
        }
        assert_eq!(tools[0]["annotations"], json!({ "readOnlyHint": true, "destructiveHint": false, "openWorldHint": true }));
        assert_eq!(tools[1]["annotations"], json!({ "readOnlyHint": true }));
        assert_eq!(tools[2]["annotations"], json!({ "readOnlyHint": false, "destructiveHint": false }));
        assert_eq!(tools[3]["annotations"], json!({ "readOnlyHint": false, "destructiveHint": true }));
        let delegate = &tools[0];
        let props = delegate["inputSchema"]["properties"].as_object().unwrap();
        assert_eq!(props.keys().map(String::as_str).collect::<Vec<_>>().len(), 3);
        assert!(props.contains_key("brief") && props.contains_key("files") && props.contains_key("read_only"));
        assert!(!props.contains_key("model") && !delegate.to_string().contains("\"model\""));
        assert_eq!(delegate["inputSchema"]["required"], json!(["brief"]));
        // The descriptions carry the guidance the planner needs.
        let d = delegate["description"].as_str().unwrap();
        assert!(d.contains("independent") && d.contains("self-contained") && d.contains("Review the returned diff") && d.contains("merge only what you verified"));
        assert!(d.contains("EVERY file") && d.contains("show_diff"));
        let show = &tools[1];
        assert_eq!(show["inputSchema"]["required"], json!(["worker_id"]));
        let props = show["inputSchema"]["properties"].as_object().unwrap();
        assert_eq!(props.keys().map(String::as_str).collect::<Vec<_>>(), ["file", "offset", "worker_id"]);
        assert_eq!(props["offset"]["type"], "integer");
        assert!(show["description"].as_str().unwrap().contains("pages"));
        let merge = &tools[2];
        assert_eq!(merge["inputSchema"]["required"], json!(["worker_id"]), "files is optional");
        assert_eq!(merge["inputSchema"]["properties"]["files"]["type"], "array");
        let m = merge["description"].as_str().unwrap();
        assert!(m.contains("after you reviewed") && m.contains("REFUSES") && m.contains("show_diff") && m.contains("files"));
        assert!(!tools.iter().any(|t| t.to_string().contains("\"model\"")), "no tool takes a model");
    }

    #[tokio::test]
    async fn delegate_runs_through_the_delegator_and_returns_the_report() {
        let (s, api) = rig();
        let r = send(&s, call(4, "delegate", json!({ "brief": "add a retry", "files": ["src/net.rs"], "read_only": false }))).await;
        assert_eq!(r["result"]["isError"], false);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("worker w1") && text.contains("added the retry") && text.contains("diff"));
        assert!(text.contains("merge(w1"));
        // Every changed file is listed with its counts, and the one not shown says how to read it.
        assert!(text.contains("src/net.rs +3 -1") && text.contains("src/big.rs +900 -0"), "{text}");
        assert!(text.contains("NOT SHOWN") && text.contains("src/big.rs") && text.contains("show_diff(w1, file)"), "{text}");
        assert_eq!(*api.delegated.lock().unwrap(), vec![DelegateRequest { brief: "add a retry".into(), files: vec!["src/net.rs".into()], read_only: false }]);
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
        assert_eq!(*api.delegated.lock().unwrap(), vec![DelegateRequest { brief: "do it".into(), files: vec![], read_only: true }]);
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

    #[test]
    fn command_line_options() {
        let (dir, planner) = parse_args(["--dir".to_string(), "/repo".into(), "--planner".into(), "claude:claude-opus-5-5".into()]).unwrap();
        assert_eq!((dir, planner.as_str()), (Some(PathBuf::from("/repo")), "claude:claude-opus-5-5"));
        assert_eq!(parse_args([]).unwrap(), (None, String::new()));
        assert!(parse_args(["--bogus".to_string()]).is_err());
        assert!(parse_args(["--dir".to_string()]).is_err());
    }
}
