//! The tool list the server advertises: names, descriptions, annotations and input schemas.
//! There is no model parameter on any tool.

use serde_json::{json, Value};

const DELEGATE_DESCRIPTION: &str = "Hand ONE self-contained piece of a larger task to a worker model. It returns a task id AT \
ONCE and the worker runs in the background: start every independent piece first, carry on with your own work, then call wait to \
collect the results. Give each call a COMPLETE, self-contained brief: the worker sees only the brief, not your conversation. There \
is no model parameter: the worker model is chosen for you.\n\
READ-ONLY pieces (read_only: true; research, finding code, explaining) run in your folder in plan mode, see your uncommitted edits, \
change nothing and return a report.\n\
EDITING pieces run in their own private git worktree cut from the last commit, so they do NOT see your uncommitted edits: commit what \
a worker needs first or put it in the brief. When the worker finishes, jev runs the project's tests in its worktree. When you call \
wait and they passed and the changes apply cleanly, they are AUTO-MERGED into your working tree and you get the report and the list \
of EVERY file changed (with +/- counts); undo(worker_id) takes them back out. Otherwise (no test command, failing tests, a conflict, \
or review: true) you get the reason and the diffs of as many files as fit: review the diff yourself and merge only what you verified. \
A file whose diff is not shown must be read with show_diff before it can be merged.";
const WAIT_DESCRIPTION: &str = "Collect delegated tasks: wait until they end, then get each one's result (and its auto-merge, \
which happens here, never while you work). Without task_ids it waits for every task not yet collected. With timeout_seconds it \
returns after that long with whatever ended and lists the rest as still running (0 checks without waiting). Each task is \
collected once.";
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
const UNDO_DESCRIPTION: &str = "Take an AUTO-MERGED worker's changes (by worker_id) back out of your working tree, exactly as they \
were applied. It refuses, changing nothing, when those files have since changed so the changes no longer come out cleanly.";

fn worker_id_schema() -> Value {
    json!({ "type": "string", "description": "The worker_id a delegate call returned." })
}

/// The input of a tool that takes only a worker id.
fn worker_id_only() -> Value {
    json!({ "type": "object", "properties": { "worker_id": worker_id_schema() }, "required": ["worker_id"] })
}

pub(super) fn tools_list() -> Value {
    json!({ "tools": [
        {
            "name": "delegate",
            "description": DELEGATE_DESCRIPTION,
            // Starting a worker changes nothing in the caller's tree: it edits only its own
            // worktree, or reads in plan mode. Anything that lands does so in `wait`.
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "openWorldHint": true },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "brief": { "type": "string", "description": "The complete, self-contained task for the worker: goal, constraints, what done looks like. It sees nothing else." },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Optional paths the worker should look at first. Hints, not limits." },
                    "read_only": { "type": "boolean", "description": "True when the piece only reads and reports (no files change). It then runs in your folder in plan mode." },
                    "review": { "type": "boolean", "description": "True to review the diff yourself even if the tests pass (for risky changes). Default: auto-merge when tests pass." }
                },
                "required": ["brief"]
            }
        },
        {
            "name": "wait",
            "description": WAIT_DESCRIPTION,
            // Auto-merges land here, in the caller's working tree.
            "annotations": { "readOnlyHint": false, "destructiveHint": false },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_ids": { "type": "array", "items": { "type": "string" }, "description": "Optional: the task ids delegate returned. Default: every task not yet collected." },
                    "timeout_seconds": { "type": "integer", "minimum": 0, "description": "Optional: return after this many seconds with what has ended. Default: wait until all have ended." }
                },
                "required": []
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
            "inputSchema": worker_id_only()
        },
        {
            "name": "undo",
            "description": UNDO_DESCRIPTION,
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": worker_id_only()
        }
    ] })
}
