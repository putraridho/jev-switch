use super::*;

mod run;
mod trust;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jev-hooks-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hooks_with(dir: &Path, json: &str) -> Hooks {
    let user = dir.join("cfg");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(user.join("hooks.json"), json).unwrap();
    Hooks::load(&Places { project: dir.to_path_buf(), user: Some(user) })
}
