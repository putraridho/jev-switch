//! Permission requests from the agent and the host's answers to them.

use serde_json::{json, Value};

use super::Acp;
use crate::backend::{Decision, Event, PermissionRequest};

impl Acp {
    pub(super) fn permission(&mut self, ev: &Value) -> Vec<Event> {
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
        let (tool, input) = match shell_command(&call) {
            Some(command) => ("Bash".to_string(), json!({ "command": command })),
            None => (
                foreign_tool_name(call["name"].as_str().or(call["title"].as_str()).unwrap_or("AntigravityTool")),
                // What the model passed to the tool, not the protocol's bookkeeping around it
                // (`kind`, `status`, `toolCallId`, a copy of the input): that is all the
                // question needs to show.
                if raw.as_object().is_some_and(|o| !o.is_empty()) { raw.clone() } else { json!({ "title": call["title"] }) },
            ),
        };
        let input = crate::approval::infrastructure::tool_input_wire::tool_input_from_json(&input);
        let summary = crate::approval::domain::review::describe(&tool, &input);
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

/// An Antigravity tool's name as the approval rules see it. A name like Claude Code's own
/// (`WebFetch`, `Write`) would be judged by that tool's rules, so a capitalized one is
/// prefixed: only the tools this module maps (a shell command) are ever judged by rules.
fn foreign_tool_name(name: &str) -> String {
    if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        format!("acp_{name}")
    } else {
        name.to_string()
    }
}

/// The command of an execute call, from the field its raw input names it by (`command`, or
/// Antigravity's `CommandLine`). A call that also names a directory to run in is not taken
/// for a plain command: the rules judge the command as run in the project.
fn shell_command(call: &Value) -> Option<&str> {
    let raw = &call["rawInput"];
    if call["kind"] != "execute" || ["Cwd", "cwd", "workdir"].iter().any(|k| raw[k].as_str().is_some_and(|d| !d.trim().is_empty())) {
        return None;
    }
    raw["command"].as_str().or(raw["CommandLine"].as_str()).filter(|c| !c.trim().is_empty())
}

fn cancelled(id: &Value) -> String {
    format!("{}\n", json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}}))
}
