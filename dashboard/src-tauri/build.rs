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
    ];
    let manifest = tauri_build::AppManifest::new().commands(commands);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
}
