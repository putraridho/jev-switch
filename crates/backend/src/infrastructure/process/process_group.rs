//! Killing the whole process group of a CLI.

/// Owns the process group of a running CLI. The CLIs spawn helpers (shells, test
/// runs, MCP servers) and `kill_on_drop` reaches only the direct child, so when the
/// turn ends badly or is cancelled (the future dropped) the whole group gets SIGKILL.
/// A turn that finished cleanly [`disarm`](Self::disarm)s it: background work an
/// agent started on purpose is not ours to kill. On Windows the process tree
/// under the CLI is killed instead.
pub(super) struct GroupKill(pub(super) Option<u32>);

impl GroupKill {
    pub(super) fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupKill {
    fn drop(&mut self) {
        // The group this runner created with `process_group(0)` on Unix; the process tree
        // under the CLI on Windows.
        if let Some(pid) = self.0 {
            jev_shared::infrastructure::platform::kill_tree(pid);
        }
    }
}

/// Alive means running: a killed process that nobody has reaped yet is a zombie, which
/// `kill(pid, 0)` still reports as there. Unix only, like the process groups it tests.
#[cfg(all(any(test, feature = "testing"), unix))]
pub fn process_alive(pid: i32) -> bool {
    // SAFETY: signal 0 delivers nothing; it only asks whether the process exists.
    let exists = unsafe { libc::kill(pid, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    exists && !is_zombie(pid)
}

/// Linux: the state letter in `/proc/<pid>/stat` (after the `(name)`, which may hold spaces).
#[cfg(all(any(test, feature = "testing"), target_os = "linux"))]
fn is_zombie(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.rsplit(')').next().unwrap_or("").trim_start().starts_with('Z'))
}

/// macOS and the BSDs have no `/proc`: ask `ps` for the state.
#[cfg(all(any(test, feature = "testing"), unix, not(target_os = "linux")))]
fn is_zombie(pid: i32) -> bool {
    std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim_start().starts_with('Z'))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::process_alive as alive;
    #[cfg(unix)]
    use std::time::Duration;

    #[cfg(unix)]
    #[test]
    fn process_alive_tells_running_from_zombie_from_gone() {
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id() as i32;
        assert!(alive(pid), "a running process is alive");
        assert!(alive(std::process::id() as i32), "so is this one");
        child.kill().unwrap();
        // Killed but not yet waited for: a zombie, which is not alive (the test helper must
        // not call it running, or the group tests would pass vacuously on any OS).
        for _ in 0..200 {
            if !alive(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!alive(pid), "a zombie is not alive");
        child.wait().unwrap();
        assert!(!alive(pid), "a reaped process is gone");
    }
}
