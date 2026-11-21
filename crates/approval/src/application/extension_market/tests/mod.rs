use super::*;
pub(super) use jev_shared::infrastructure::system_host::SystemHost;
use crate::application::extension_market::manifest::pack;

mod install;
mod listing;
mod manifest;
mod recovery;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jev-market-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn package(dir: &Path, version: &str, body: &str) {
    std::fs::create_dir_all(dir.join("commands")).unwrap();
    std::fs::write(dir.join("commands/hello.md"), body).unwrap();
    pack(dir, "hello-kit", version).unwrap();
}
