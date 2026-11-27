//! Engine boundary for the coding-agent chat tab: the ONLY file in `jev-chat-io`
//! that imports the engine facade.

use jev_chat_core::ipc::{
    ChatCatalogModel, ChatDecision, ChatEdit, ChatEvent, ChatMode, ChatPermissionPrompt, ChatPin,
    ChatProviderStatus, ChatRouteInfo, ChatToolCall, ChatTurnStatus, ChatUsage,
};
use jev_engine as engine;
use jev_usage_core::port::SourceId;
use jev_usage_core::provider::ProviderKey;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::{broadcast, mpsc, oneshot};

/// The `ToolActivity` summary that carries the result of the tool row before it.
pub const TOOL_RESULT: &str = "Tool result";
/// The `ToolActivity` summary that carries the edit diff of the tool row before it.
pub const TOOL_EDIT: &str = "Tool edit";

/// Re-exported handle to the underlying engine.
pub type EngineHandle = engine::Engine;
/// Re-exported turn cancellation token.
pub type CancelToken = engine::Cancel;
/// Pending permission sender stored by request id.
pub type PermissionMap = Arc<Mutex<HashMap<String, oneshot::Sender<engine::Decision>>>>;

/// Converts an engine `ProviderId` to a Flashboard `ProviderKey`.
pub fn to_provider_key(id: engine::ProviderId) -> ProviderKey {
    match id {
        engine::ProviderId::Claude => ProviderKey::Claude,
        engine::ProviderId::Codex => ProviderKey::Codex,
        engine::ProviderId::Gemini => ProviderKey::Gemini,
        engine::ProviderId::Grok => ProviderKey::Grok,
        engine::ProviderId::DeepSeek => ProviderKey::DeepSeek,
    }
}

/// Converts a Flashboard `ProviderKey` to an engine `ProviderId`. `Antigravity` maps to
/// `Gemini` because the engine routes Antigravity quota through `ProviderId::Gemini`.
pub fn to_provider_id(key: ProviderKey) -> engine::ProviderId {
    match key {
        ProviderKey::Claude => engine::ProviderId::Claude,
        ProviderKey::Codex => engine::ProviderId::Codex,
        ProviderKey::Gemini | ProviderKey::Antigravity => engine::ProviderId::Gemini,
        ProviderKey::Grok => engine::ProviderId::Grok,
        ProviderKey::DeepSeek => engine::ProviderId::DeepSeek,
    }
}

/// Converts a UI `ChatMode` to engine `Mode`.
pub fn to_engine_mode(mode: ChatMode) -> engine::Mode {
    match mode {
        ChatMode::Ask => engine::Mode::Ask,
        ChatMode::Edit => engine::Mode::Edit,
        ChatMode::Plan => engine::Mode::Plan,
        ChatMode::Auto => engine::Mode::Auto,
    }
}

/// Converts a UI `ChatPin` to engine `Pin`.
pub fn to_engine_pin(pin: &ChatPin) -> engine::Pin {
    match pin {
        ChatPin::Auto => engine::Pin::Auto,
        ChatPin::Provider { provider } => engine::Pin::Provider(to_provider_id(*provider)),
        ChatPin::Model { provider, model } => {
            engine::Pin::Model(to_provider_id(*provider), model.clone())
        }
    }
}

/// Converts a UI `ChatDecision` to engine `Decision`.
pub fn to_engine_decision(decision: ChatDecision) -> engine::Decision {
    match decision {
        ChatDecision::Allow => engine::Decision::Allow,
        ChatDecision::AllowAlways => engine::Decision::AllowAlways,
        ChatDecision::Deny => engine::Decision::Deny {
            reason: String::from("Denied by user in Flashboard"),
        },
    }
}

/// Converts engine `TurnStatus` to IPC `ChatTurnStatus`.
pub fn to_chat_status(status: engine::TurnStatus) -> ChatTurnStatus {
    match status {
        engine::TurnStatus::Answered => ChatTurnStatus::Answered,
        engine::TurnStatus::Failed => ChatTurnStatus::Failed,
        engine::TurnStatus::Cancelled => ChatTurnStatus::Cancelled,
        engine::TurnStatus::NoRoute => ChatTurnStatus::NoRoute,
    }
}

/// Result of building a desktop engine for a workspace.
pub struct BuiltEngine {
    /// Configured engine.
    pub engine: EngineHandle,
    /// Whether `JEV_API_KEY` / `TYPESAFE_API_KEY` was detected.
    pub jev_key_present: bool,
}

/// Builds a full multi-provider interactive coding `Engine` rooted at `dir`.
pub async fn build_engine(
    dir: PathBuf,
    mode: ChatMode,
    orchestrate: bool,
    pin: &ChatPin,
    routing_enabled: bool,
) -> BuiltEngine {
    let built = engine::build_desktop_engine(
        dir,
        to_engine_mode(mode),
        orchestrate,
        to_engine_pin(pin),
        routing_enabled,
    )
    .await;
    BuiltEngine {
        engine: built.engine,
        jev_key_present: built.jev_key_present,
    }
}

/// Reads provider availability from the engine runtime.
pub fn provider_statuses(eng: &EngineHandle) -> Vec<ChatProviderStatus> {
    engine::provider_readiness(eng)
        .into_iter()
        .map(|p| ChatProviderStatus {
            provider: to_provider_key(p.provider),
            ready: p.ready,
            detail: p.detail,
            plan: p.plan,
        })
        .collect()
}

/// Lists all catalog models for UI selection.
pub fn catalog_models() -> Vec<ChatCatalogModel> {
    engine::catalog_models()
        .into_iter()
        .map(|(provider, id)| ChatCatalogModel {
            provider: to_provider_key(provider),
            id: id.to_owned(),
        })
        .collect()
}

/// Extracts summarized report fields after a turn completes.
pub struct TurnSummary {
    /// Terminal turn status.
    pub status: ChatTurnStatus,
    /// Answer text (or error message if not answered).
    pub text: String,
    /// Provider that answered or failed last.
    pub provider: Option<ProviderKey>,
    /// Model id.
    pub model: Option<String>,
    /// Files modified in the workspace.
    pub changed_files: Vec<String>,
    /// Token usage counts, when reported.
    pub usage: Option<ChatUsage>,
}

/// Summarizes an engine `TurnReport` into IPC-friendly fields.
pub fn summarize_report(report: &engine::TurnReport) -> TurnSummary {
    let status = to_chat_status(report.status);
    let text = if matches!(status, ChatTurnStatus::Answered) {
        report.text.clone()
    } else if !report.message.is_empty() {
        report.message.clone()
    } else {
        String::from("Turn did not produce an answer.")
    };
    let changed_files = report
        .changes
        .as_ref()
        .map(|cs| cs.files.iter().map(|f| f.path.clone()).collect())
        .unwrap_or_default();
    let usage = report.usage.as_ref().map(|u| ChatUsage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        cache_read_tokens: u.cached_input_tokens,
    });
    TurnSummary {
        status,
        text,
        provider: report.provider.map(to_provider_key),
        model: report.model.clone(),
        changed_files,
        usage,
    }
}

/// Runs a cancellable turn and forwards `Ui` events to the desktop UI.
pub async fn run_turn(
    eng: &mut EngineHandle,
    prompt: &str,
    ui: &mut DesktopChatUi,
    cancel: &CancelToken,
) -> TurnSummary {
    let report = eng.run_cancellable(prompt, ui, cancel).await;
    summarize_report(&report)
}

/// Shared callback state updated by `DesktopChatUi` during a live turn.
pub trait ChatSink: Send + Sync {
    /// Records and broadcasts a `ChatEvent`.
    fn emit(&self, event: ChatEvent);
}

/// Implements `jev_turn::application::ports::Ui` for Flashboard's integrated chat tab.
pub struct DesktopChatUi {
    sink: Arc<dyn ChatSink>,
    pending_perms: PermissionMap,
    usage_signal: Option<mpsc::Sender<SourceId>>,
    next_perm_seq: u64,
}

impl DesktopChatUi {
    /// Creates a UI adapter forwarding events to `sink`, storing pending permission
    /// resolvers in `pending_perms`, and optionally signalling `usage_signal`.
    pub fn new(
        sink: Arc<dyn ChatSink>,
        pending_perms: PermissionMap,
        usage_signal: Option<mpsc::Sender<SourceId>>,
    ) -> Self {
        Self {
            sink,
            pending_perms,
            usage_signal,
            next_perm_seq: 0,
        }
    }

    fn signal_usage(&self) {
        if let Some(tx) = &self.usage_signal {
            let _ = tx.try_send(SourceId::ClaudeGatewayLog);
        }
    }
}

impl engine::Ui for DesktopChatUi {
    fn route(&mut self, line: &str) {
        self.sink.emit(ChatEvent::Note {
            line: line.to_owned(),
        });
    }

    fn route_facts(&mut self, facts: &engine::RouteFacts) {
        let provider = facts
            .provider
            .parse::<engine::ProviderId>()
            .map_or(ProviderKey::Claude, to_provider_key);
        self.sink.emit(ChatEvent::RouteChosen {
            route: ChatRouteInfo {
                provider,
                model: facts.model.clone(),
                task: facts.task.clone(),
                reason: facts.reason.clone(),
                switched_provider: facts.switched_provider,
            },
        });
    }

    fn event(&mut self, e: &engine::Event) {
        match e {
            engine::Event::TextDelta(chunk) => {
                self.sink.emit(ChatEvent::TextDelta {
                    text: chunk.clone(),
                });
            }
            engine::Event::Thinking(chunk) => {
                self.sink.emit(ChatEvent::ThinkingDelta {
                    text: chunk.clone(),
                });
            }
            engine::Event::Tool(summary) => {
                self.sink.emit(ChatEvent::ToolActivity {
                    tool: ChatToolCall {
                        summary: summary.clone(),
                        ok: None,
                        output_preview: None,
                        edit: None,
                    },
                });
            }
            engine::Event::ToolResult { ok, text } => {
                let preview: String = text.chars().take(240).collect();
                self.sink.emit(ChatEvent::ToolActivity {
                    tool: ChatToolCall {
                        summary: String::from(TOOL_RESULT),
                        ok: Some(*ok),
                        output_preview: (!preview.is_empty()).then_some(preview),
                        edit: None,
                    },
                });
            }
            engine::Event::Edit {
                path,
                removed,
                added,
            } => {
                self.sink.emit(ChatEvent::ToolActivity {
                    tool: ChatToolCall {
                        summary: String::from(TOOL_EDIT),
                        ok: None,
                        output_preview: None,
                        edit: Some(ChatEdit {
                            path: path.clone(),
                            removed: removed.clone(),
                            added: added.clone(),
                        }),
                    },
                });
            }
            engine::Event::Hint(text) => {
                self.sink.emit(ChatEvent::Note { line: text.clone() });
            }
            engine::Event::Plan(_) | engine::Event::Finished(_) => {
                self.signal_usage();
            }
            engine::Event::Failed { message } => {
                self.sink.emit(ChatEvent::Note {
                    line: message.clone(),
                });
            }
            engine::Event::Text(_)
            | engine::Event::Steered(_)
            | engine::Event::PermissionRequest(_)
            | engine::Event::ElicitationRequest(_)
            | engine::Event::PermissionWithdrawn { .. } => {}
        }
    }

    fn note(&mut self, line: &str) {
        for part in line.lines().filter(|l| !l.trim().is_empty()) {
            self.sink.emit(ChatEvent::Note {
                line: part.to_owned(),
            });
        }
    }

    fn failover(&mut self, f: &engine::Failover) {
        self.sink.emit(ChatEvent::Note {
            line: f.text.to_owned(),
        });
    }

    fn no_route(&mut self, _info: &engine::NoRouteInfo, text: &str) {
        self.sink.emit(ChatEvent::Note {
            line: text.to_owned(),
        });
    }

    fn cancelled(&mut self, text: &str) {
        if !text.trim().is_empty() {
            self.sink.emit(ChatEvent::Note {
                line: text.to_owned(),
            });
        }
    }

    fn ask_permission(
        &mut self,
        req: &engine::PermissionRequest,
        offer: &engine::Offer,
    ) -> engine::Answer {
        self.next_perm_seq = self.next_perm_seq.saturating_add(1);
        let request_id = format!("perm_{}_{}", self.next_perm_seq, req.id);
        let (tx, rx) = oneshot::channel::<engine::Decision>();
        {
            let mut guard = self
                .pending_perms
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            guard.insert(request_id.clone(), tx);
        }
        let (can_always, always_label) = match offer {
            Ok(always) => (true, Some(always.label())),
            Err(_) => (false, None),
        };
        let prompt = ChatPermissionPrompt {
            request_id: request_id.clone(),
            tool: req.tool.clone(),
            summary: req.summary.clone(),
            blocked_path: req.blocked_path.clone(),
            can_always,
            always_label,
        };
        self.sink.emit(ChatEvent::PermissionRequested { prompt });
        let map = Arc::clone(&self.pending_perms);
        let sink = Arc::clone(&self.sink);
        Box::pin(async move {
            let answer = rx.await.ok();
            {
                let mut guard = map.lock().unwrap_or_else(PoisonError::into_inner);
                guard.remove(&request_id);
            }
            sink.emit(ChatEvent::PermissionCleared { request_id });
            answer
        })
    }

    fn permission_settled(&mut self, req: &engine::PermissionRequest, settled: &engine::Settled) {
        if let engine::Settled::NotSaved(err) = settled {
            self.sink.emit(ChatEvent::Note {
                line: format!("[permissions] {}: {err}; allowed once", req.tool),
            });
        }
    }

    fn permission_withdrawn(&mut self, req: &engine::PermissionRequest) {
        let mut removed = Vec::new();
        {
            let mut guard = self
                .pending_perms
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let suffix = format!("_{}", req.id);
            let keys: Vec<String> = guard
                .keys()
                .filter(|k| k.ends_with(&suffix))
                .cloned()
                .collect();
            for key in keys {
                guard.remove(&key);
                removed.push(key);
            }
        }
        for request_id in removed {
            self.sink.emit(ChatEvent::PermissionCleared { request_id });
        }
    }
}

/// Resolves a pending permission request by `request_id`. Returns `true` if found.
pub fn resolve_permission(
    pending_perms: &PermissionMap,
    request_id: &str,
    decision: ChatDecision,
) -> bool {
    let tx = {
        let mut guard = pending_perms.lock().unwrap_or_else(PoisonError::into_inner);
        guard.remove(request_id)
    };
    match tx {
        Some(sender) => sender.send(to_engine_decision(decision)).is_ok(),
        None => false,
    }
}

/// Subscribes a `broadcast::Sender<ChatEvent>` for typed downstream listeners.
pub fn subscribe_channel(tx: &broadcast::Sender<ChatEvent>) -> broadcast::Receiver<ChatEvent> {
    tx.subscribe()
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

    struct RecordingSink {
        events: Mutex<Vec<ChatEvent>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }

        fn take(&self) -> Vec<ChatEvent> {
            std::mem::take(&mut *self.events.lock().unwrap())
        }
    }

    impl ChatSink for RecordingSink {
        fn emit(&self, event: ChatEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[test]
    fn permission_map_resolves_and_drops_unknown() {
        let map: PermissionMap = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel();
        map.lock().unwrap().insert("req_1".to_owned(), tx);

        assert!(resolve_permission(&map, "req_1", ChatDecision::AllowAlways));
        assert!(!resolve_permission(&map, "req_1", ChatDecision::Deny));
        assert_eq!(rx.blocking_recv().unwrap(), engine::Decision::AllowAlways);
    }

    #[test]
    fn desktop_ui_forwards_text_tools_and_notes() {
        use engine::Ui;

        let sink = Arc::new(RecordingSink::new());
        let map: PermissionMap = Arc::new(Mutex::new(HashMap::new()));
        let mut ui = DesktopChatUi::new(sink.clone(), map, None);

        ui.route("routed line");
        ui.event(&engine::Event::TextDelta("hello".to_owned()));
        ui.event(&engine::Event::Tool("Bash cargo check".to_owned()));
        ui.event(&engine::Event::ToolResult {
            ok: true,
            text: "done".to_owned(),
        });
        ui.note("line 1\n\nline 2");

        let recorded = sink.take();
        assert_eq!(recorded.len(), 6);
        assert!(matches!(&recorded[0], ChatEvent::Note { line } if line == "routed line"));
        assert!(matches!(&recorded[1], ChatEvent::TextDelta { text } if text == "hello"));
        assert!(
            matches!(&recorded[2], ChatEvent::ToolActivity { tool } if tool.summary == "Bash cargo check")
        );
        assert!(matches!(&recorded[3], ChatEvent::ToolActivity { tool } if tool.ok == Some(true)));
        assert!(matches!(&recorded[4], ChatEvent::Note { line } if line == "line 1"));
        assert!(matches!(&recorded[5], ChatEvent::Note { line } if line == "line 2"));
    }
}
