//! Key loading (DESIGN 8.5): a key is read at fetch time, used once and dropped. The types
//! are frozen; T05 writes the bodies.

use crate::{engine_bridge, envfile};
use jev_usage_core::secret::Secret;
use std::path::PathBuf;
use zeroize::Zeroizing;

/// Environment variable and env-file key name for DeepSeek.
const DEEPSEEK_KEY_VAR: &str = "DEEPSEEK_API_KEY";

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
        let env = std::env::var(DEEPSEEK_KEY_VAR).ok().and_then(|value| {
            let mut value = Zeroizing::new(value);
            (!value.trim().is_empty()).then(|| Secret::new(std::mem::take(&mut *value)))
        });
        Self {
            env,
            env_file: engine_bridge::env_file(),
        }
    }
}

/// Environment first, else the env file through `io::envfile::read_value`. By value: a
/// `Secret` is not `Clone`, so the environment's key is moved out, never copied.
pub fn deepseek_key(sources: KeySources) -> Option<Secret<String>> {
    if let Some(key) = sources.env {
        return Some(key);
    }
    let path = sources.env_file?;
    let mut value = envfile::read_value(&path, DEEPSEEK_KEY_VAR)?;
    if value.trim().is_empty() {
        return None;
    }
    Some(Secret::new(std::mem::take(&mut *value)))
}
