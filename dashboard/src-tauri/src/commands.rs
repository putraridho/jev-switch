//! The six commands (DESIGN 7.3): exactly these, with names fixed by the capability file.
//! Every command resolves with the generated envelope `Outcome<T>`; none rejects on purpose.
//! The outer `Result` of the async ones exists only because Tauri demands one from an async
//! command that borrows state; it is always `Ok`. T11 writes the bodies.

use jev_usage_core::ipc::{
    CommandError, Diagnostics, HistoryQuery, HistorySeries, Outcome, RefreshTarget, Settings,
    SettingsPatch, Snapshot,
};
use jev_usage_io::service::ServiceHandle;
use tauri::{AppHandle, Runtime, State};

/// The latest snapshot; always `ok`.
#[tauri::command]
pub fn get_snapshot(svc: State<'_, ServiceHandle>) -> Outcome<Snapshot> {
    let _ = svc;
    Outcome::Error {
        error: CommandError::Internal,
    }
}

/// Queues a refresh and returns; the result arrives as a `snapshot` event.
#[tauri::command]
pub async fn refresh(
    svc: State<'_, ServiceHandle>,
    target: RefreshTarget,
) -> Result<Outcome<()>, CommandError> {
    let _ = (svc, target);
    Ok(Outcome::Error {
        error: CommandError::Internal,
    })
}

/// One meter's history: at most 600 points.
#[tauri::command]
pub async fn get_history(
    svc: State<'_, ServiceHandle>,
    query: HistoryQuery,
) -> Result<Outcome<HistorySeries>, CommandError> {
    let _ = (svc, query);
    Ok(Outcome::Error {
        error: CommandError::Internal,
    })
}

/// The current settings; always `ok`.
#[tauri::command]
pub fn get_settings(svc: State<'_, ServiceHandle>) -> Outcome<Settings> {
    let _ = svc;
    Outcome::Error {
        error: CommandError::Internal,
    }
}

/// Applies a patch (validated in core); the shell then reconciles the autostart entry.
#[tauri::command]
pub async fn update_settings<R: Runtime>(
    app: AppHandle<R>,
    svc: State<'_, ServiceHandle>,
    patch: SettingsPatch,
) -> Result<Outcome<Settings>, CommandError> {
    let _ = (app, svc, patch);
    Ok(Outcome::Error {
        error: CommandError::Internal,
    })
}

/// Version, per-source state and store size.
#[tauri::command]
pub async fn get_diagnostics<R: Runtime>(
    app: AppHandle<R>,
    svc: State<'_, ServiceHandle>,
) -> Result<Outcome<Diagnostics>, CommandError> {
    let _ = (app, svc);
    Ok(Outcome::Error {
        error: CommandError::Internal,
    })
}
