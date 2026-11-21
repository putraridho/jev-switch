//! The real [`Host`]: the process environment and key file, the system clock, the state
//! directory under `$XDG_STATE_HOME`, and the files and git calls of this machine.

use std::path::{Path, PathBuf};

use crate::shared::application::host::Host;

use super::{env_file, git_guard, jsonl_log, private_file};

pub struct SystemHost;

impl Host for SystemHost {
    fn env(&self, name: &str) -> Option<String> {
        env_file::lookup(name)
    }

    fn home(&self) -> Option<String> {
        std::env::var("HOME").ok()
    }

    fn process_id(&self) -> u32 {
        std::process::id()
    }

    fn now_ms(&self) -> u64 {
        jsonl_log::now_ms()
    }

    fn state_dir(&self) -> Option<PathBuf> {
        jsonl_log::state_dir()
    }

    fn read_text(&self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn write_private(&self, path: &Path, text: &str) -> std::io::Result<()> {
        private_file::write_atomic(path, text)
    }

    fn append_line(&self, path: &str, text: &str) {
        jsonl_log::append_line(path, text);
    }

    fn no_repo_programs(&self, dir: &Path) -> Vec<String> {
        git_guard::no_repo_programs(dir)
    }
}
