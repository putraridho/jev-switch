//! Declares the six commands as the app's manifest (DESIGN 8.8), so each one needs an
//! explicit permission in `capabilities/main.json` and no other command can be granted.

fn main() -> impl std::process::Termination {
    let commands = &[
        "get_snapshot",
        "refresh",
        "get_history",
        "get_settings",
        "update_settings",
        "get_diagnostics",
        "chat_get_state",
        "chat_send",
        "chat_cancel",
        "chat_set_workspace",
        "chat_set_mode",
        "chat_set_pin",
        "chat_set_orchestrate",
        "chat_answer_permission",
        "chat_new_session",
        "chat_list_dirs",
        "chat_attach_data",
        "chat_attach_text",
    ];
    let manifest = tauri_build::AppManifest::new().commands(commands);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
}
