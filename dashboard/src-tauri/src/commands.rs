//! The six commands (DESIGN 7.3): exactly these, with names fixed by the capability file.
//! Every command resolves with the generated envelope `Outcome<T>`; none rejects on purpose.
//! The outer `Result` of the async ones exists only because Tauri demands one from an async
//! command that borrows state; it is always `Ok`. T11 writes the bodies.

use crate::bus::TrayHost;
use jev_chat_core::ipc::{
    ChatDirListing, ChatMode, ChatPermissionReply, ChatPin, ChatSessionState,
};
use jev_chat_io::service::ChatServiceHandle;
use jev_usage_core::ipc::{
    CommandError, Diagnostics, HistoryQuery, HistorySeries, Outcome, RefreshTarget, Settings,
    SettingsPatch, Snapshot,
};
use jev_usage_io::service::{ServiceHandle, ShellFacts};
use tauri::{AppHandle, Runtime, State};

/// The latest snapshot; always `ok`.
#[tauri::command]
pub fn get_snapshot(svc: State<'_, ServiceHandle>) -> Outcome<Snapshot> {
    crate::window::first_snapshot_served();
    Outcome::Ok {
        value: svc.snapshot(),
    }
}

/// Queues a refresh and returns; the result arrives as a `snapshot` event.
#[tauri::command]
pub async fn refresh(
    svc: State<'_, ServiceHandle>,
    target: RefreshTarget,
) -> Result<Outcome<()>, CommandError> {
    Ok(Outcome::from(svc.refresh(target).await))
}

/// One meter's history: at most 600 points.
#[tauri::command]
pub async fn get_history(
    svc: State<'_, ServiceHandle>,
    query: HistoryQuery,
) -> Result<Outcome<HistorySeries>, CommandError> {
    Ok(Outcome::from(svc.history(query).await))
}

/// The current settings; always `ok`.
#[tauri::command]
pub fn get_settings(svc: State<'_, ServiceHandle>) -> Outcome<Settings> {
    Outcome::Ok {
        value: svc.settings(),
    }
}

/// Applies a patch (validated in core); the shell then reconciles the autostart entry.
#[tauri::command]
pub async fn update_settings<R: Runtime>(
    app: AppHandle<R>,
    svc: State<'_, ServiceHandle>,
    patch: SettingsPatch,
) -> Result<Outcome<Settings>, CommandError> {
    let result = svc.update_settings(patch).await;
    if let Ok(settings) = &result {
        crate::autostart::apply(settings.autostart);
        crate::window::apply_zoom(&app, settings.zoom_pct);
    }
    Ok(Outcome::from(result))
}

/// Version, per-source state and store size.
#[tauri::command]
pub async fn get_diagnostics<R: Runtime>(
    _app: AppHandle<R>,
    svc: State<'_, ServiceHandle>,
) -> Result<Outcome<Diagnostics>, CommandError> {
    let facts = ShellFacts {
        app_version: env!("CARGO_PKG_VERSION"),
        tray_available: crate::bus::SessionBusTrayHost.present(),
    };
    Ok(Outcome::Ok {
        value: svc.diagnostics(facts).await,
    })
}

/// Current coding-agent chat session state; always `ok`.
#[tauri::command]
pub fn chat_get_state(chat: State<'_, ChatServiceHandle>) -> Outcome<ChatSessionState> {
    Outcome::Ok {
        value: chat.state(),
    }
}

/// Starts a routed coding-agent turn for `prompt`.
#[tauri::command]
pub async fn chat_send(
    chat: State<'_, ChatServiceHandle>,
    prompt: String,
) -> Result<Outcome<ChatSessionState>, CommandError> {
    Ok(Outcome::from(chat.send_prompt(prompt).await))
}

/// Cancels the active turn if running.
#[tauri::command]
pub fn chat_cancel(chat: State<'_, ChatServiceHandle>) -> Outcome<ChatSessionState> {
    Outcome::Ok {
        value: chat.cancel_turn(),
    }
}

/// Switches the active workspace directory.
#[tauri::command]
pub async fn chat_set_workspace(
    chat: State<'_, ChatServiceHandle>,
    path: String,
) -> Result<Outcome<ChatSessionState>, CommandError> {
    Ok(Outcome::from(chat.set_workspace(path).await))
}

/// Sets the tool permission mode.
#[tauri::command]
pub async fn chat_set_mode(
    chat: State<'_, ChatServiceHandle>,
    mode: ChatMode,
) -> Result<Outcome<ChatSessionState>, CommandError> {
    Ok(Outcome::from(chat.set_mode(mode).await))
}

/// Sets the model routing pin.
#[tauri::command]
pub async fn chat_set_pin(
    chat: State<'_, ChatServiceHandle>,
    pin: ChatPin,
) -> Result<Outcome<ChatSessionState>, CommandError> {
    Ok(Outcome::from(chat.set_pin(pin).await))
}

/// Toggles `/orchestrate` delegation.
#[tauri::command]
pub async fn chat_set_orchestrate(
    chat: State<'_, ChatServiceHandle>,
    on: bool,
) -> Result<Outcome<ChatSessionState>, CommandError> {
    Ok(Outcome::from(chat.set_orchestrate(on).await))
}

/// Answers a pending tool permission request.
#[tauri::command]
pub fn chat_answer_permission(
    chat: State<'_, ChatServiceHandle>,
    reply: ChatPermissionReply,
) -> Outcome<()> {
    Outcome::from(chat.answer_permission(reply))
}

/// Starts a fresh coding-agent conversation.
#[tauri::command]
pub async fn chat_new_session(
    chat: State<'_, ChatServiceHandle>,
) -> Result<Outcome<ChatSessionState>, CommandError> {
    Ok(Outcome::from(chat.new_session().await))
}

/// Lists subdirectories for the workspace folder picker.
#[tauri::command]
pub fn chat_list_dirs(
    chat: State<'_, ChatServiceHandle>,
    path: Option<String>,
) -> Outcome<ChatDirListing> {
    Outcome::from(chat.list_dirs(path))
}
