//! Long prompts handed to a CLI as a private file, and the files removed again.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::domain::prompt_arg::{PromptArg, MAX_PROMPT_ARG_BYTES};

/// Create and verify the fallback directory without following a symlink.
fn create_private_prompt_dir(dir: &Path) -> std::io::Result<()> {
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        let mut builder = builder;
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
    };
    match builder.create(dir) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(dir)?;
    let private = metadata.file_type().is_dir();
    #[cfg(unix)]
    let private = {
        use std::os::unix::fs::MetadataExt;
        private && metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0
    };
    if !private {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "prompt directory is not private"));
    }
    Ok(())
}

fn prompt_arg_in_fallback(dir: &Path, prompt: &str) -> PromptArg {
    if prompt.len() <= MAX_PROMPT_ARG_BYTES || create_private_prompt_dir(dir).is_err() {
        return PromptArg::Inline(prompt.to_string());
    }
    prompt_arg_in(dir, prompt)
}

/// A new, uniquely named file in `dir` (created if missing), readable by the owner only.
fn write_private(dir: &Path, content: &str) -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    std::fs::create_dir_all(dir)?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let name = format!("prompt-{}-{nanos}-{}.txt", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let path = dir.join(name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&path)?, content.as_bytes())?;
    Ok(path)
}

/// The one rule for a prompt that must travel as a command-line argument (Grok,
/// Antigravity): the prompt itself up to [`MAX_PROMPT_ARG_BYTES`], else a private
/// file in `dir` plus a short instruction. The caller must remove the file, see
/// [`PromptFiles`]. If the file cannot be written the prompt is passed inline anyway,
/// so the CLI's own error is what the user sees rather than a silent loss.
pub fn prompt_arg_in(dir: &Path, prompt: &str) -> PromptArg {
    if prompt.len() <= MAX_PROMPT_ARG_BYTES {
        return PromptArg::Inline(prompt.to_string());
    }
    match write_private(dir, prompt) {
        Ok(path) => {
            let instruction = format!(
                "Your task is written in the file {}. Read that file completely and carry out what it says.",
                path.display()
            );
            PromptArg::File { path, instruction }
        }
        Err(_) => PromptArg::Inline(prompt.to_string()),
    }
}

/// [`prompt_arg_in`] in the state directory, or a verified private temp directory.
pub fn prompt_arg(prompt: &str) -> PromptArg {
    match jev_shared::infrastructure::jsonl_log::state_dir() {
        Some(dir) => prompt_arg_in(&dir.join("prompts"), prompt),
        None => prompt_arg_in_fallback(&std::env::temp_dir().join("jev-prompts"), prompt),
    }
}

/// Owns the prompt files one backend instance wrote, so they are removed when the
/// turn ends ([`Backend::cleanup`]) and, as a safety net, when the backend is dropped
/// (a turn cancelled mid-way). The files can hold code and must never be left behind.
#[derive(Default)]
pub struct PromptFiles {
    dir: Option<PathBuf>,
    written: Mutex<Vec<PathBuf>>,
}

impl PromptFiles {
    /// Write into `dir` instead of the default directory.
    pub fn in_dir(dir: PathBuf) -> Self {
        Self { dir: Some(dir), written: Mutex::default() }
    }

    /// [`prompt_arg_in`], remembering the file for [`PromptFiles::cleanup`].
    pub fn arg(&self, prompt: &str) -> PromptArg {
        let arg = match &self.dir {
            Some(dir) => prompt_arg_in(dir, prompt),
            None => prompt_arg(prompt),
        };
        if let Some(path) = arg.file() {
            self.written.lock().unwrap_or_else(PoisonError::into_inner).push(path.to_path_buf());
        }
        arg
    }

    /// Remove every file written so far.
    pub fn cleanup(&self) {
        for path in self.written.lock().unwrap_or_else(PoisonError::into_inner).drain(..) {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for PromptFiles {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-prompt-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_prompt_under_the_limit_is_passed_through_unchanged() {
        let dir = scratch_dir("short");
        let prompt = "x".repeat(MAX_PROMPT_ARG_BYTES);
        let arg = prompt_arg_in(&dir, &prompt);
        assert_eq!(arg, PromptArg::Inline(prompt.clone()));
        assert_eq!(arg.text(), prompt);
        assert!(arg.file().is_none());
        assert!(!dir.exists(), "no file, not even the directory, is made for a short prompt");
    }

    #[cfg(unix)]
    #[test]
    fn fallback_prompt_directory_must_be_private_and_not_a_symlink() {
        use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

        let dir = scratch_dir("fallback");
        let prompt = "x".repeat(MAX_PROMPT_ARG_BYTES + 1);
        let arg = prompt_arg_in_fallback(&dir, &prompt);
        assert!(arg.file().is_some());
        let metadata = std::fs::symlink_metadata(&dir).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o077, 0);

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(prompt_arg_in_fallback(&dir, &prompt), PromptArg::Inline(prompt.clone()));
        std::fs::remove_dir_all(&dir).unwrap();

        let target = scratch_dir("fallback-target");
        create_private_prompt_dir(&target).unwrap();
        symlink(&target, &dir).unwrap();
        assert_eq!(prompt_arg_in_fallback(&dir, &prompt), PromptArg::Inline(prompt.clone()));
        std::fs::remove_file(&dir).unwrap();
        std::fs::remove_dir(&target).unwrap();

        std::fs::write(&dir, "not a directory").unwrap();
        assert_eq!(prompt_arg_in_fallback(&dir, &prompt), PromptArg::Inline(prompt));
        std::fs::remove_file(&dir).unwrap();
    }

    #[test]
    fn a_long_prompt_goes_to_a_private_file_and_the_arg_is_a_short_instruction() {
        let dir = scratch_dir("long");
        let prompt = format!("{}\u{e9}end", "y".repeat(MAX_PROMPT_ARG_BYTES));
        let arg = prompt_arg_in(&dir, &prompt);
        let path = arg.file().expect("long prompt is written to a file").to_path_buf();
        assert!(arg.text().len() < 500, "{}", arg.text());
        assert!(arg.text().contains(&path.display().to_string()));
        assert!(arg.text().contains("Read that file completely"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), prompt);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Two prompts never share a file.
        let other = prompt_arg_in(&dir, &prompt);
        assert_ne!(other.file(), Some(path.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompt_files_cleanup_removes_what_was_written_and_drop_is_a_safety_net() {
        let dir = scratch_dir("cleanup");
        let long = "z".repeat(MAX_PROMPT_ARG_BYTES + 1);
        let files = PromptFiles::in_dir(dir.clone());
        let a = files.arg(&long).file().unwrap().to_path_buf();
        assert!(a.exists());
        files.cleanup();
        assert!(!a.exists());
        let b = files.arg(&long).file().unwrap().to_path_buf();
        drop(files);
        assert!(!b.exists(), "dropping the owner removes its files too");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
