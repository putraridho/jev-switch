//! Typed IPC models for the integrated coding-agent chat tab.

use jev_usage_core::provider::ProviderKey;
use serde::{Deserialize, Serialize};

/// Permission mode controlling tool and edit approvals during a turn.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ChatMode {
    /// Ask before every edit and command.
    Ask,
    /// Accept file edits, ask before shell commands.
    #[default]
    Edit,
    /// Read and plan only; no file or shell mutations.
    Plan,
    /// Accept edits and tool calls without asking.
    Auto,
}

/// Optional manual override for model routing.
#[derive(Clone, Eq, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatPin {
    /// Route automatically across usable providers.
    #[default]
    Auto,
    /// Restrict routing to one provider's models.
    Provider {
        /// The pinned provider.
        provider: ProviderKey,
    },
    /// Pin one exact model id.
    Model {
        /// The provider owning the model.
        provider: ProviderKey,
        /// The catalog model id.
        model: String,
    },
}

/// Speaker role of a transcript entry.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    /// Prompt sent by the user.
    User,
    /// Reply from an assistant model.
    Assistant,
    /// Engine diagnostic or status note.
    System,
}

/// Terminal outcome of one turn.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ChatTurnStatus {
    /// Answered cleanly.
    Answered,
    /// Ended with an error.
    Failed,
    /// Stopped by the user.
    Cancelled,
    /// No eligible model could take the turn.
    NoRoute,
}

/// User decision for a pending tool permission request.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ChatDecision {
    /// Allow this single tool invocation.
    Allow,
    /// Allow and persist the rule/folder when offered.
    AllowAlways,
    /// Deny the tool invocation.
    Deny,
}

/// A line diff preview emitted by an edit tool call.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatEdit {
    /// Target file path.
    pub path: String,
    /// Removed lines.
    pub removed: Vec<String>,
    /// Added lines.
    pub added: Vec<String>,
}

/// One tool call observed during a turn.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatToolCall {
    /// Human-readable one-line summary (e.g. `Bash cargo test`).
    pub summary: String,
    /// Result flag once completed (`None` while running or if not reported).
    pub ok: Option<bool>,
    /// Short preview of tool output, if any.
    pub output_preview: Option<String>,
    /// File edit diff preview, if any.
    pub edit: Option<ChatEdit>,
}

/// Routing decision selected for the active or completed turn.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatRouteInfo {
    /// Chosen provider.
    pub provider: ProviderKey,
    /// Chosen model identifier.
    pub model: String,
    /// Task category reported by Jev.
    pub task: String,
    /// Plain-English routing rationale.
    pub reason: String,
    /// Whether this turn switched away from the previous turn's provider.
    pub switched_provider: bool,
}

/// Token counts reported for a completed turn.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatUsage {
    /// Input tokens consumed.
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub input_tokens: u64,
    /// Output tokens produced.
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub output_tokens: u64,
    /// Tokens read from prompt cache.
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub cache_read_tokens: u64,
}

/// What an attachment is, which decides how it reaches the model.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ChatAttachmentKind {
    /// A picture (PNG, JPEG, GIF, WebP): sent to the model as an image.
    Image,
    /// Text, code or a PDF: the model is given its path to read.
    File,
    /// Text pasted into the composer, kept as a file so the prompt stays short.
    Text,
}

/// A file going with a message. The path is absolute; pasted content is staged in the app's
/// state folder first, so every attachment is a file on disk by the time it is sent.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatAttachment {
    /// The name shown on its chip.
    pub name: String,
    /// Absolute path of the file.
    pub path: String,
    /// How it reaches the model.
    pub kind: ChatAttachmentKind,
    /// Size in bytes.
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub bytes: u64,
    /// Line count, for text (pasted text shows "84 lines" rather than a size).
    #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
    pub lines: Option<u64>,
}

/// The answer to one file offered as an attachment: taken, or refused with the reason the
/// chip shows ("212 MB · over 20 MB").
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ChatAttachResult {
    /// The file can go with the message.
    Ready {
        /// What will be sent.
        attachment: ChatAttachment,
    },
    /// The file can't be attached.
    Refused {
        /// The file's name.
        name: String,
        /// Size in bytes, when it could be read.
        #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
        bytes: Option<u64>,
        /// Why, in a few words: "over 20 MB", "not a file", "can't read this kind of file".
        reason: String,
    },
}

/// One entry in the chat transcript.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatMessage {
    /// Monotonic entry id within the session.
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub id: u64,
    /// Who produced the entry.
    pub role: ChatRole,
    /// Markdown or plain text body.
    pub content: String,
    /// Answering provider, for assistant messages.
    pub provider: Option<ProviderKey>,
    /// Answering model id, for assistant messages.
    pub model: Option<String>,
    /// Routing summary attached to this turn.
    pub route_reason: Option<String>,
    /// Tool calls executed during this turn.
    pub tool_calls: Vec<ChatToolCall>,
    /// Collected reasoning summary, if any.
    pub thinking: Option<String>,
    /// Files modified in the workspace during this turn.
    pub changed_files: Vec<String>,
    /// Outcome of the turn.
    pub status: ChatTurnStatus,
    /// Token usage, when reported.
    pub usage: Option<ChatUsage>,
    /// Files sent with a user message, in the order attached.
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
}

/// Pending tool permission prompt waiting for user confirmation.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatPermissionPrompt {
    /// Unique request id to echo in [`ChatPermissionReply`].
    pub request_id: String,
    /// Tool name (e.g. `Bash`, `Edit`).
    pub tool: String,
    /// Command or file path summary.
    pub summary: String,
    /// Path outside allowed folders, when applicable.
    pub blocked_path: Option<String>,
    /// Whether "Allow always" is offered by policy.
    pub can_always: bool,
    /// Label explaining what "Allow always" saves.
    pub always_label: Option<String>,
}

/// User response to a pending tool permission prompt.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatPermissionReply {
    /// Matching [`ChatPermissionPrompt::request_id`].
    pub request_id: String,
    /// Chosen action.
    pub decision: ChatDecision,
}

/// Provider availability as reported by the routing runtime.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatProviderStatus {
    /// Provider key.
    pub provider: ProviderKey,
    /// True when ready to run turns.
    pub ready: bool,
    /// Short diagnostic text.
    pub detail: String,
    /// Active plan name when known.
    pub plan: Option<String>,
}

/// Model option offered in the model pin menu.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatCatalogModel {
    /// Owning provider.
    pub provider: ProviderKey,
    /// Model id.
    pub id: String,
}

/// Listing of subdirectories for the built-in workspace folder picker.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatDirListing {
    /// Canonical path of the directory being listed.
    pub current: String,
    /// Parent directory path, if any.
    pub parent: Option<String>,
    /// Immediate visible subdirectories (sorted).
    pub dirs: Vec<String>,
}

/// Full state snapshot of the desktop coding-agent chat tab.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ChatSessionState {
    /// Active workspace folder path.
    pub workspace: String,
    /// Current permission mode.
    pub mode: ChatMode,
    /// Current model pin.
    pub pin: ChatPin,
    /// Whether multi-provider task routing is enabled.
    pub routing_enabled: bool,
    /// Whether `/orchestrate` delegation is enabled.
    pub orchestrate: bool,
    /// True when `JEV_API_KEY` or `TYPESAFE_API_KEY` is configured.
    pub jev_key_present: bool,
    /// A short title a model wrote for the session after its first reply; `None` until then
    /// (or when no model could write one), and the UI falls back to the first prompt.
    pub title: Option<String>,
    /// True while a turn is currently running.
    pub busy: bool,
    /// Active route chosen for the in-flight turn.
    pub active_route: Option<ChatRouteInfo>,
    /// Assistant answer text streamed so far in the active turn.
    pub streaming_text: String,
    /// Reasoning text streamed so far in the active turn.
    pub streaming_thinking: String,
    /// Tool calls observed so far in the active turn.
    pub streaming_tools: Vec<ChatToolCall>,
    /// Status/failover notes for the active turn.
    pub notes: Vec<String>,
    /// Active tool approval waiting on user input.
    pub pending_permission: Option<ChatPermissionPrompt>,
    /// Completed transcript entries.
    pub messages: Vec<ChatMessage>,
    /// Live provider readiness list.
    pub providers: Vec<ChatProviderStatus>,
    /// Catalog models available for pinning.
    pub models: Vec<ChatCatalogModel>,
}

/// Live events emitted from Rust to the chat UI over `"chat_event"`.
#[derive(Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatEvent {
    /// Full state synchronization.
    State {
        /// Latest state.
        state: Box<ChatSessionState>,
    },
    /// Streamed assistant text delta.
    TextDelta {
        /// Appended text chunk.
        text: String,
    },
    /// Streamed reasoning text delta.
    ThinkingDelta {
        /// Appended reasoning chunk.
        text: String,
    },
    /// Route selected for the current attempt.
    RouteChosen {
        /// Route details.
        route: ChatRouteInfo,
    },
    /// Tool call started or updated.
    ToolActivity {
        /// Tool call event.
        tool: ChatToolCall,
    },
    /// Diagnostic or failover note.
    Note {
        /// Line text.
        line: String,
    },
    /// Tool call waiting on user decision.
    PermissionRequested {
        /// Prompt details.
        prompt: ChatPermissionPrompt,
    },
    /// Pending permission was settled or withdrawn.
    PermissionCleared {
        /// Cleared request id.
        request_id: String,
    },
}
