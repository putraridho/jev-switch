//! Key loading (DESIGN 8.5): a key is read at fetch time, used once and dropped. The types
//! are frozen; T05 writes the bodies.

use jev_usage_core::secret::Secret;
use std::path::PathBuf;

/// Where a key may come from. Injected, so tests never touch the real environment or home.
pub struct KeySources {
    /// The key from the process environment, if set.
    pub env: Option<Secret<String>>,
    /// The env file to read otherwise.
    pub env_file: Option<PathBuf>,
}

impl KeySources {
    /// The process environment and `engine_bridge::env_file()`.
    pub fn system() -> Self {
        Self {
            env: None,
            env_file: None,
        }
    }
}

/// Environment first, else the env file through `io::envfile::read_value`. By value: a
/// `Secret` is not `Clone`, so the environment's key is moved out, never copied.
pub fn deepseek_key(sources: KeySources) -> Option<Secret<String>> {
    let _ = sources;
    None
}
