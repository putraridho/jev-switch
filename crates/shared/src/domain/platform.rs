//! How this platform names and signals programs: pure facts the callers act on.

/// The negative process-group id `kill(2)` takes to signal a whole group. `None` for a pid that
/// would signal something else: `kill(-0)` is `kill(0)`, our own group.
#[cfg(any(test, unix))]
pub fn group_signal_target(pid: u32) -> Option<i32> {
    i32::try_from(pid).ok().filter(|p| *p > 0).map(|p| -p)
}

/// Whether `name` is a path (it has a directory part) rather than a bare program name.
pub fn is_path(name: &str) -> bool {
    name.contains('/') || (cfg!(windows) && name.contains('\\'))
}

/// The file names a program called `name` may have on this platform: just `name` on Unix; on
/// Windows also `name` with each executable extension (`claude` is `claude.exe` or `claude.cmd`).
pub fn executable_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        executable_names_with(name, &[".exe", ".cmd", ".bat", ".com"])
    } else {
        vec![name.to_string()]
    }
}

/// [`executable_names`] for an explicit list of extensions: `name` itself first (it may
/// already carry one), then `name` plus each extension that it does not already end in.
pub fn executable_names_with(name: &str, extensions: &[&str]) -> Vec<String> {
    let lower = name.to_ascii_lowercase();
    let mut names = vec![name.to_string()];
    names.extend(extensions.iter().filter(|e| !lower.ends_with(*e)).map(|e| format!("{name}{e}")));
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_names_add_each_extension_once() {
        assert_eq!(executable_names_with("claude", &[".exe", ".cmd"]), vec!["claude", "claude.exe", "claude.cmd"]);
        assert_eq!(executable_names_with("claude.EXE", &[".exe", ".cmd"]), vec!["claude.EXE", "claude.EXE.cmd"], "already has .exe");
        assert!(!executable_names("claude").is_empty());
    }

    #[test]
    fn the_group_signal_never_targets_our_own_group_or_everything() {
        assert_eq!(group_signal_target(4242), Some(-4242));
        assert_eq!(group_signal_target(0), None);
        assert_eq!(group_signal_target(u32::MAX), None);
    }

    #[test]
    fn a_path_has_a_directory_part() {
        assert!(is_path("/usr/bin/claude") && is_path("./claude"));
        assert!(!is_path("claude"));
        assert_eq!(is_path("C:\\bin\\claude.exe"), cfg!(windows));
    }
}
