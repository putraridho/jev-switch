//! The wiring (DESIGN 8.8, TASKS T11): the service, the tray, the signal listener and the
//! window lifecycle.

use crate::{cli, commands, hardening, tray, window};
use jev_usage_core::ipc::Route;
use jev_usage_core::port::SourceId;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

/// The label of the one window; the capability file grants it and nothing else.
pub const MAIN_WINDOW: &str = "main";
/// The product name: window title, tray tooltip, notifications (DESIGN 1a).
pub const PRODUCT_NAME: &str = "Flashboard";

/// Builds the service dependencies and starts the service; returns the usage signal sender
/// when set up so the chat service can notify usage refresh on completed turns.
fn start_service(app: &tauri::AppHandle) -> Option<tokio::sync::mpsc::Sender<SourceId>> {
    let state_dir = jev_usage_io::engine_bridge::state_dir();
    let dashboard_dir = state_dir
        .as_deref()
        .map(|d| d.join("dashboard"))
        .or_else(|| Some(std::env::temp_dir().join("jev-dashboard")));

    let dashboard_dir = dashboard_dir?;
    let _ = jev_usage_io::fsutil::ensure_private_dir(&dashboard_dir);

    let store_path = dashboard_dir.join("history.db");
    let settings_path = dashboard_dir.join("settings.json");
    let usage_log = jev_usage_io::engine_bridge::usage_log_path();

    let Ok(store) = jev_usage_io::store::SqliteStore::open(&store_path) else {
        return None;
    };
    let store = Arc::new(store);
    let settings = Arc::new(jev_usage_io::settings_file::JsonSettingsStore::new(
        settings_path,
    ));

    let clock: Arc<dyn jev_usage_core::time::Clock> = Arc::new(jev_usage_io::clock::SystemClock);
    let timer: Arc<dyn jev_usage_io::timer::WallTimer> =
        Arc::new(jev_usage_io::timer::SystemTimer::new());
    let jitter: Arc<dyn jev_usage_io::service::Jitter> =
        Arc::new(jev_usage_io::service::SystemJitter);

    let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let runner = Arc::new(jev_usage_io::proc::ProcRunner::system(workdir));
    let probe: Arc<dyn jev_usage_io::probe::LoginProbe> =
        Arc::new(jev_usage_io::probe::CliProbe::new(Arc::clone(&runner)));
    let facts = jev_usage_io::probe::FactsCell::new(jev_usage_io::probe::LoginFacts::unknown());

    let Ok(http) = jev_usage_io::http::HttpClient::new() else {
        return None;
    };
    let http = Arc::new(http);
    let keys: Arc<dyn Fn() -> jev_usage_io::secrets::KeySources + Send + Sync> =
        Arc::new(jev_usage_io::secrets::KeySources::system);

    let inputs = jev_usage_io::registry::RegistryInputs {
        usage_log: usage_log.clone(),
        facts: Arc::clone(&facts),
        clock: Arc::clone(&clock),
        runner: Arc::clone(&runner),
        http: Arc::clone(&http),
        keys,
    };
    let sources = jev_usage_io::registry::build(&inputs);
    let backfill = jev_usage_io::registry::backfill_job(usage_log.clone(), Arc::clone(&store));

    let notifier: Arc<dyn jev_usage_core::port::Notifier> =
        Arc::new(crate::notifier::DesktopNotifier::new(
            Arc::new(crate::localtime::SystemLocalTime),
            Arc::clone(&clock),
            Arc::new(crate::notifier::NotifyRustSink),
        ));

    let (signal_tx, signal_rx) = tokio::sync::mpsc::channel::<SourceId>(64);
    let chat_signal = signal_tx.clone();

    let deps = jev_usage_io::service::ServiceDeps {
        clock,
        timer,
        jitter,
        sources,
        probe,
        facts,
        store,
        settings,
        notifier,
        signals: signal_rx,
        backfill,
    };

    let runtime_handle = tauri::async_runtime::handle();
    let rt = runtime_handle.inner();
    let handle = jev_usage_io::service::spawn(deps, rt);
    crate::events::forward_snapshots(app.clone(), handle.subscribe());
    app.manage(handle);

    if let Some(log) = usage_log {
        let guard = jev_usage_io::watch::watch_file(
            &log,
            SourceId::ClaudeGatewayLog,
            signal_tx,
            jev_usage_io::watch::WatchOptions::default(),
            rt,
        );
        app.manage(guard);
    }
    Some(chat_signal)
}

fn start_chat_service(
    app: &tauri::AppHandle,
    usage_signal: Option<tokio::sync::mpsc::Sender<SourceId>>,
) {
    let workspace = jev_chat_io::service::default_workspace(
        std::env::current_dir().ok(),
        std::env::var_os("HOME").map(PathBuf::from),
    );
    let runtime_handle = tauri::async_runtime::handle();
    let rt = runtime_handle.inner().clone();
    let handle = jev_chat_io::service::ChatServiceHandle::start(workspace, usage_signal, rt);
    crate::events::forward_chat_events(app.clone(), handle.subscribe());
    app.manage(handle);
}

/// SIGTERM ends the app through its normal exit, so the service is shut down and any
/// subprocess it started (killed with its process group) does not outlive it.
fn exit_on_sigterm(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        else {
            return;
        };
        term.recv().await;
        handle.exit(0);
    });
}

/// Runs the app until it exits.
pub fn run() {
    hardening::apply();
    let args = cli::parse(std::env::args().skip(1));
    let built = tauri::Builder::default()
        // Registered first: a second process ends here, before anything else is created.
        // Its arguments and working directory are dropped, never stored or logged.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args.iter().any(|a| a == "--hidden") {
                window::open(app, Route::Overview);
            }
        }))
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::refresh,
            commands::get_history,
            commands::get_settings,
            commands::update_settings,
            commands::get_diagnostics,
            commands::chat_get_state,
            commands::chat_send,
            commands::chat_cancel,
            commands::chat_set_workspace,
            commands::chat_set_mode,
            commands::chat_set_pin,
            commands::chat_set_orchestrate,
            commands::chat_answer_permission,
            commands::chat_new_session,
            commands::chat_list_dirs,
            commands::chat_attach_data,
            commands::chat_attach_text
        ])
        .setup(move |app| {
            let usage_signal = start_service(app.handle());
            start_chat_service(app.handle(), usage_signal);
            if let Some(svc) = app.try_state::<jev_usage_io::service::ServiceHandle>() {
                crate::autostart::apply(svc.settings().autostart);
            }
            let _ = tray::install(app.handle());
            if !args.hidden {
                window::open(app.handle(), Route::Overview);
            }
            exit_on_sigterm(app.handle().clone());
            jev_usage_io::diag::emit(jev_usage_io::diag::DiagEvent::Started);
            Ok(())
        })
        .build(tauri::generate_context!());
    let Ok(app) = built else {
        std::process::exit(1);
    };
    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            jev_usage_io::diag::emit(jev_usage_io::diag::DiagEvent::Stopping);
            if let Some(svc) = handle.try_state::<jev_usage_io::service::ServiceHandle>() {
                tauri::async_runtime::block_on(svc.shutdown());
            }
        }
    });
}
