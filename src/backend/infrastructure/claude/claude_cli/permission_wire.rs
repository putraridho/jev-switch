//! Host approvals as the CLI sends them: `control_request` and `control_cancel_request`.

use serde_json::Value;

use crate::approval::domain::review;
use crate::backend::{Event, PermissionRequest};

/// The host's question from a `can_use_tool` control request; other control requests are not ours.
pub(super) fn permission_request(ev: &Value) -> Option<PermissionRequest> {
    let request = &ev["request"];
    if request["subtype"] != "can_use_tool" {
        return None;
    }
    let tool = request["tool_name"].as_str()?;
    let input = request["input"].clone();
    Some(PermissionRequest {
        id: ev["request_id"].as_str()?.to_string(),
        tool: tool.to_string(),
        summary: review::describe(tool, &input),
        blocked_path: request["blocked_path"].as_str().map(String::from),
        input,
    })
}

/// A `control_cancel_request`: the CLI no longer needs the answer to `request_id`. Its shape is
/// `{"type":"control_cancel_request","request_id":"..."}` (Claude Agent SDK control protocol;
/// the schema is in the CLI binary). It may name any control request, not only a permission
/// question; the runner ignores an id that is not waiting.
pub(super) fn withdrawn_request(ev: &Value) -> Option<Event> {
    Some(Event::PermissionWithdrawn { id: ev["request_id"].as_str()?.to_string() })
}
