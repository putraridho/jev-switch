//! Stateful coding-agent chat service for Flashboard's integrated Chat tab.

use crate::engine_bridge::{
    build_engine, catalog_models, provider_statuses, resolve_permission, run_turn, to_engine_mode,
    to_engine_pin, CancelToken, ChatSink, DesktopChatUi, EngineHandle, PermissionMap, TOOL_EDIT,
    TOOL_RESULT,
};
use jev_chat_core::ipc::{
    ChatDirListing, ChatEvent, ChatMessage, ChatMode, ChatPermissionReply, ChatPin, ChatRole,
    ChatSessionState, ChatToolCall, ChatTurnStatus,
};
use jev_usage_core::ipc::CommandError;
use jev_usage_core::port::SourceId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex};

/// The folder the chat opens in. A launch from Finder or the Dock starts in `/`, which is
/// never a project, so that (or no folder at all) falls back to the home folder. Inside a git
/// repository the chat opens at the repository root rather than the subfolder it was
/// started from (a dev run starts in `dashboard/src-tauri`).
pub fn default_workspace(cwd: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    let start = match cwd {
        Some(dir) if dir.parent().is_some() => dir,
        _ => return home.unwrap_or_else(|| PathBuf::from("/")),
    };
    start
        .ancestors()
        .find(|dir| dir.join(".git").exists() && Some(*dir) != home.as_deref())
        .map_or_else(|| start.clone(), Path::to_path_buf)
}

/// Shared broadcast sink updating `ChatSessionState` and pushing live events.
struct SharedSink {
    state: Arc<Mutex<ChatSessionState>>,
    events: broadcast::Sender<ChatEvent>,
}

impl ChatSink for SharedSink {
    fn emit(&self, event: ChatEvent) {
        {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            match &event {
                ChatEvent::State { state } => *st = (**state).clone(),
                ChatEvent::TextDelta { text } => st.streaming_text.push_str(text),
                ChatEvent::ThinkingDelta { text } => st.streaming_thinking.push_str(text),
                ChatEvent::RouteChosen { route } => st.active_route = Some(route.clone()),
                ChatEvent::ToolActivity { tool } => merge_tool(&mut st.streaming_tools, tool),
                ChatEvent::Note { line } => st.notes.push(line.clone()),
                ChatEvent::PermissionRequested { prompt } => {
                    st.pending_permission = Some(prompt.clone());
                }
                ChatEvent::PermissionCleared { request_id } => {
                    if st
                        .pending_permission
                        .as_ref()
                        .is_some_and(|p| p.request_id == *request_id)
                    {
                        st.pending_permission = None;
                    }
                }
            }
        }
        let _ = self.events.send(event);
    }
}

/// Folds one tool event into the running list: a result or an edit diff belongs to the
/// tool row before it (an edit is not done until its result arrives, so it never sets `ok`).
fn merge_tool(tools: &mut Vec<ChatToolCall>, tool: &ChatToolCall) {
    match (tool.summary.as_str(), tools.last_mut()) {
        (TOOL_RESULT, Some(last)) => {
            last.ok = tool.ok;
            last.output_preview.clone_from(&tool.output_preview);
        }
        (TOOL_EDIT, Some(last)) => last.edit.clone_from(&tool.edit),
        (TOOL_EDIT, None) => tools.push(ChatToolCall {
            summary: tool
                .edit
                .as_ref()
                .map_or_else(|| String::from("Edit"), |e| format!("Edit {}", e.path)),
            ..tool.clone()
        }),
        _ => tools.push(tool.clone()),
    }
}

/// Handle held in Tauri state to drive the coding-agent chat tab.
#[derive(Clone)]
pub struct ChatServiceHandle {
    state: Arc<Mutex<ChatSessionState>>,
    engine: Arc<AsyncMutex<Option<EngineHandle>>>,
    cancel: Arc<Mutex<Option<CancelToken>>>,
    pending_perms: PermissionMap,
    events: broadcast::Sender<ChatEvent>,
    sink: Arc<dyn ChatSink>,
    usage_signal: Option<mpsc::Sender<SourceId>>,
    next_msg_id: Arc<Mutex<u64>>,
    rt: tokio::runtime::Handle,
    /// Bumped by every new session or folder change, so a late title is not put on the
    /// wrong conversation.
    session_gen: Arc<AtomicU64>,
    /// Set once a title has been asked for in this session.
    title_asked: Arc<AtomicBool>,
    /// The `claude` binary the title is asked through.
    title_bin: Arc<str>,
}

impl ChatServiceHandle {
    /// Creates the service handle initialized at `workspace` (lazy engine boot in background).
    pub fn start(
        workspace: PathBuf,
        usage_signal: Option<mpsc::Sender<SourceId>>,
        rt: tokio::runtime::Handle,
    ) -> Self {
        let initial = ChatSessionState {
            workspace: workspace.to_string_lossy().into_owned(),
            mode: ChatMode::Edit,
            pin: ChatPin::Auto,
            routing_enabled: true,
            orchestrate: false,
            jev_key_present: false,
            title: None,
            busy: false,
            active_route: None,
            streaming_text: String::new(),
            streaming_thinking: String::new(),
            streaming_tools: Vec::new(),
            notes: Vec::new(),
            pending_permission: None,
            messages: Vec::new(),
            providers: Vec::new(),
            models: catalog_models(),
        };
        let state = Arc::new(Mutex::new(initial));
        let (events, _) = broadcast::channel::<ChatEvent>(256);
        let sink = Arc::new(SharedSink {
            state: Arc::clone(&state),
            events: events.clone(),
        });
        let engine = Arc::new(AsyncMutex::new(None));
        let handle = Self {
            state,
            engine,
            cancel: Arc::new(Mutex::new(None)),
            pending_perms: Arc::new(Mutex::new(HashMap::new())),
            events,
            sink,
            usage_signal,
            next_msg_id: Arc::new(Mutex::new(0)),
            rt: rt.clone(),
            session_gen: Arc::new(AtomicU64::new(0)),
            title_asked: Arc::new(AtomicBool::new(false)),
            title_bin: Arc::from(crate::title::claude_bin()),
        };
        let init_handle = handle.clone();
        rt.spawn(async move {
            let _ = init_handle.ensure_engine().await;
        });
        handle
    }

    /// Uses `bin` instead of `claude` for session titles (tests).
    #[must_use]
    pub fn with_title_bin(mut self, bin: &str) -> Self {
        self.title_bin = Arc::from(bin);
        self
    }

    /// Forgets the session's title, and makes any title still being written arrive too late.
    fn reset_title(&self, st: &mut ChatSessionState) {
        st.title = None;
        self.session_gen.fetch_add(1, Ordering::SeqCst);
        self.title_asked.store(false, Ordering::SeqCst);
    }

    /// After the session's first answered reply, asks a small model for a title in the
    /// background. Once per session; the first-prompt title stays until it arrives.
    fn maybe_request_title(&self, st: &ChatSessionState) {
        if st.title.is_some() {
            return;
        }
        let Some(reply) = st
            .messages
            .iter()
            .rev()
            .find(|m| m.role == ChatRole::Assistant && m.status == ChatTurnStatus::Answered)
        else {
            return;
        };
        let Some(prompt) = st.messages.iter().find(|m| m.role == ChatRole::User) else {
            return;
        };
        if self.title_asked.swap(true, Ordering::SeqCst) {
            return;
        }
        let (prompt, reply) = (prompt.content.clone(), reply.content.clone());
        let generation = self.session_gen.load(Ordering::SeqCst);
        let service = self.clone();
        self.rt.spawn(async move {
            let cwd = std::env::temp_dir();
            let title =
                crate::title::generate_title(&service.title_bin, &cwd, &prompt, &reply).await;
            let Some(title) = title else { return };
            {
                let mut st = service.state.lock().unwrap_or_else(PoisonError::into_inner);
                if service.session_gen.load(Ordering::SeqCst) != generation || st.title.is_some() {
                    return;
                }
                st.title = Some(title);
            }
            service.emit_full_state();
        });
    }

    /// Subscribes to live `ChatEvent` stream.
    pub fn subscribe(&self) -> broadcast::Receiver<ChatEvent> {
        self.events.subscribe()
    }

    /// Returns the latest session state snapshot immediately.
    pub fn state(&self) -> ChatSessionState {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn next_id(&self) -> u64 {
        let mut guard = self
            .next_msg_id
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *guard = guard.saturating_add(1);
        *guard
    }

    fn emit_full_state(&self) {
        let snapshot = self.state();
        let _ = self.events.send(ChatEvent::State {
            state: Box::new(snapshot),
        });
    }

    async fn ensure_engine(&self) -> Result<(), CommandError> {
        let mut eng_guard = self.engine.lock().await;
        if eng_guard.is_some() {
            return Ok(());
        }
        let (dir, mode, orchestrate, pin, routing_enabled) = {
            let st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            (
                PathBuf::from(&st.workspace),
                st.mode,
                st.orchestrate,
                st.pin.clone(),
                st.routing_enabled,
            )
        };
        let built = build_engine(dir, mode, orchestrate, &pin, routing_enabled).await;
        let providers = provider_statuses(&built.engine);
        *eng_guard = Some(built.engine);
        {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            st.jev_key_present = built.jev_key_present;
            st.providers = providers;
        }
        self.emit_full_state();
        Ok(())
    }

    /// Starts a coding-agent turn for `prompt`. Returns immediately once spawned; progress
    /// arrives over `chat_event`.
    pub async fn send_prompt(&self, prompt: String) -> Result<ChatSessionState, CommandError> {
        let trimmed = prompt.trim().to_owned();
        if trimmed.is_empty() {
            return Err(CommandError::InvalidArgument);
        }
        {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if st.busy {
                return Err(CommandError::Busy);
            }
            st.busy = true;
            st.active_route = None;
            st.streaming_text.clear();
            st.streaming_thinking.clear();
            st.streaming_tools.clear();
            st.notes.clear();
            st.pending_permission = None;
            let user_id = self.next_id();
            st.messages.push(ChatMessage {
                id: user_id,
                role: ChatRole::User,
                content: trimmed.clone(),
                provider: None,
                model: None,
                route_reason: None,
                tool_calls: Vec::new(),
                thinking: None,
                changed_files: Vec::new(),
                status: ChatTurnStatus::Answered,
                usage: None,
            });
        }
        self.emit_full_state();

        let cancel = CancelToken::new();
        {
            let mut c = self.cancel.lock().unwrap_or_else(PoisonError::into_inner);
            *c = Some(cancel.clone());
        }

        let service = self.clone();
        self.rt.spawn(async move {
            let _ = service.ensure_engine().await;
            let mut eng_guard = service.engine.lock().await;
            let Some(eng) = eng_guard.as_mut() else {
                let mut st = service.state.lock().unwrap_or_else(PoisonError::into_inner);
                st.busy = false;
                service.emit_full_state();
                return;
            };
            let sink = Arc::clone(&service.sink);
            let mut ui = DesktopChatUi::new(
                sink,
                Arc::clone(&service.pending_perms),
                service.usage_signal.clone(),
            );
            let summary = run_turn(eng, &trimmed, &mut ui, &cancel).await;
            let providers = provider_statuses(eng);
            drop(eng_guard);

            {
                let mut c = service
                    .cancel
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                *c = None;
            }
            {
                let mut st = service.state.lock().unwrap_or_else(PoisonError::into_inner);
                let tool_calls = std::mem::take(&mut st.streaming_tools);
                let thinking_raw = std::mem::take(&mut st.streaming_thinking);
                let thinking = (!thinking_raw.trim().is_empty()).then_some(thinking_raw);
                let route_reason = st.active_route.as_ref().map(|r| r.reason.clone());
                let provider = summary
                    .provider
                    .or_else(|| st.active_route.as_ref().map(|r| r.provider));
                let model = summary
                    .model
                    .or_else(|| st.active_route.as_ref().map(|r| r.model.clone()));
                // A stopped or failed turn keeps what had already streamed in.
                let partial = std::mem::take(&mut st.streaming_text);
                let content =
                    if summary.status == ChatTurnStatus::Answered || partial.trim().is_empty() {
                        summary.text
                    } else {
                        format!("{}\n\n{}", partial.trim_end(), summary.text)
                    };
                st.active_route = None;
                st.pending_permission = None;
                st.busy = false;
                st.providers = providers;
                let msg_id = service.next_id();
                st.messages.push(ChatMessage {
                    id: msg_id,
                    role: ChatRole::Assistant,
                    content,
                    provider,
                    model,
                    route_reason,
                    tool_calls,
                    thinking,
                    changed_files: summary.changed_files,
                    status: summary.status,
                    usage: summary.usage,
                });
                service.maybe_request_title(&st);
            }
            if let Some(tx) = &service.usage_signal {
                let _ = tx.try_send(SourceId::ClaudeGatewayLog);
            }
            service.emit_full_state();
        });

        Ok(self.state())
    }

    /// Cancels the active turn if one is running.
    pub fn cancel_turn(&self) -> ChatSessionState {
        let cancel = {
            self.cancel
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        };
        if let Some(token) = cancel {
            token.cancel();
        }
        self.state()
    }

    /// Answers a pending tool approval request.
    pub fn answer_permission(&self, reply: ChatPermissionReply) -> Result<(), CommandError> {
        if resolve_permission(&self.pending_perms, &reply.request_id, reply.decision) {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if st
                .pending_permission
                .as_ref()
                .is_some_and(|p| p.request_id == reply.request_id)
            {
                st.pending_permission = None;
            }
            drop(st);
            self.emit_full_state();
            Ok(())
        } else {
            Err(CommandError::InvalidArgument)
        }
    }

    /// Changes the active workspace directory and rebuilds the engine (only when idle).
    pub async fn set_workspace(&self, path: String) -> Result<ChatSessionState, CommandError> {
        let dir = PathBuf::from(path.trim());
        if !dir.is_dir() {
            return Err(CommandError::InvalidArgument);
        }
        {
            let st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if st.busy {
                return Err(CommandError::Busy);
            }
        }
        let canonical = std::fs::canonicalize(&dir).unwrap_or(dir);
        let (mode, orchestrate, pin, routing_enabled) = {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            st.workspace = canonical.to_string_lossy().into_owned();
            st.messages.clear();
            self.reset_title(&mut st);
            st.notes.clear();
            st.active_route = None;
            (st.mode, st.orchestrate, st.pin.clone(), st.routing_enabled)
        };
        let built = build_engine(canonical, mode, orchestrate, &pin, routing_enabled).await;
        let providers = provider_statuses(&built.engine);
        {
            let mut eng_guard = self.engine.lock().await;
            *eng_guard = Some(built.engine);
        }
        {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            st.jev_key_present = built.jev_key_present;
            st.providers = providers;
        }
        self.emit_full_state();
        Ok(self.state())
    }

    /// Updates permission mode (`Ask`, `Edit`, `Plan`, `Auto`).
    pub async fn set_mode(&self, mode: ChatMode) -> Result<ChatSessionState, CommandError> {
        {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            st.mode = mode;
        }
        if let Ok(mut eng_guard) = self.engine.try_lock() {
            if let Some(eng) = eng_guard.as_mut() {
                eng.set_mode(to_engine_mode(mode));
            }
        }
        self.emit_full_state();
        Ok(self.state())
    }

    /// Updates model routing pin (`Auto`, `Provider`, or `Model`).
    pub async fn set_pin(&self, pin: ChatPin) -> Result<ChatSessionState, CommandError> {
        {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            st.pin = pin.clone();
        }
        if let Ok(mut eng_guard) = self.engine.try_lock() {
            if let Some(eng) = eng_guard.as_mut() {
                eng.set_pin(to_engine_pin(&pin));
            }
        }
        self.emit_full_state();
        Ok(self.state())
    }

    /// Toggles `/orchestrate` delegation.
    pub async fn set_orchestrate(&self, on: bool) -> Result<ChatSessionState, CommandError> {
        {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            st.orchestrate = on;
        }
        if let Ok(mut eng_guard) = self.engine.try_lock() {
            if let Some(eng) = eng_guard.as_mut() {
                eng.set_orchestrate(on);
            }
        }
        self.emit_full_state();
        Ok(self.state())
    }

    /// Clears the current conversation (`/new`).
    pub async fn new_session(&self) -> Result<ChatSessionState, CommandError> {
        {
            let st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if st.busy {
                return Err(CommandError::Busy);
            }
        }
        {
            let mut eng_guard = self.engine.lock().await;
            if let Some(eng) = eng_guard.as_mut() {
                eng.new_session();
            }
        }
        {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            st.messages.clear();
            self.reset_title(&mut st);
            st.notes.clear();
            st.streaming_text.clear();
            st.streaming_thinking.clear();
            st.streaming_tools.clear();
            st.active_route = None;
            st.pending_permission = None;
        }
        self.emit_full_state();
        Ok(self.state())
    }

    /// Lists visible subdirectories under `path` (or the current workspace when `None`).
    pub fn list_dirs(&self, path: Option<String>) -> Result<ChatDirListing, CommandError> {
        let target = match path {
            Some(p) if !p.trim().is_empty() => PathBuf::from(p.trim()),
            _ => PathBuf::from(&self.state().workspace),
        };
        let canonical =
            std::fs::canonicalize(&target).map_err(|_| CommandError::InvalidArgument)?;
        if !canonical.is_dir() {
            return Err(CommandError::InvalidArgument);
        }
        let parent = canonical.parent().map(|p| p.to_string_lossy().into_owned());
        let mut dirs = Vec::new();
        let entries = std::fs::read_dir(&canonical).map_err(|_| CommandError::Io)?;
        for entry in entries.flatten() {
            let child = entry.path();
            if !child.is_dir() {
                continue;
            }
            let Some(name) = child.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            dirs.push(child.to_string_lossy().into_owned());
        }
        dirs.sort();
        Ok(ChatDirListing {
            current: canonical.to_string_lossy().into_owned(),
            parent,
            dirs,
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;
    use jev_chat_core::ipc::ChatEdit;

    fn row(summary: &str, ok: Option<bool>, edit: Option<ChatEdit>) -> ChatToolCall {
        ChatToolCall {
            summary: summary.to_owned(),
            ok,
            output_preview: None,
            edit,
        }
    }

    #[test]
    fn an_edit_waits_for_its_result_on_the_same_row() {
        let edit = ChatEdit {
            path: "a.txt".into(),
            removed: vec![],
            added: vec!["hi".into()],
        };
        let mut tools = Vec::new();
        merge_tool(&mut tools, &row("Write a.txt", None, None));
        merge_tool(&mut tools, &row(TOOL_EDIT, None, Some(edit.clone())));
        assert_eq!(tools.len(), 1);
        assert_eq!(
            tools[0].ok, None,
            "not done before the approval and the result"
        );
        assert_eq!(tools[0].edit.as_ref(), Some(&edit));
        merge_tool(&mut tools, &row(TOOL_RESULT, Some(false), None));
        assert_eq!((tools.len(), tools[0].ok), (1, Some(false)));
    }

    #[test]
    fn an_edit_with_no_tool_row_gets_its_own() {
        let edit = ChatEdit {
            path: "b.rs".into(),
            removed: vec![],
            added: vec![],
        };
        let mut tools = Vec::new();
        merge_tool(&mut tools, &row(TOOL_EDIT, None, Some(edit)));
        assert_eq!(tools[0].summary, "Edit b.rs");
    }
}
