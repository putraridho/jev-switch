use std::path::PathBuf;

#[test]
fn every_export_has_its_contract_signature() {
    let _: fn() -> Option<PathBuf> = jev_embed::state_dir;
    let _: fn(Option<String>, Option<String>) -> Option<PathBuf> = jev_embed::env_file_path;
    let _: fn(&str, &str) -> Option<String> = jev_embed::parse_env;
    let _: fn(bool, &str) -> Option<bool> = jev_embed::parse_codex_login;
    let _: fn(jev_embed::ProviderId) -> Option<&'static str> = jev_embed::login_command;
}
