//! Real process liveness: `/proc` where there is one, else `kill -0`.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::orchestration::application::ports::Liveness;
use crate::orchestration::domain::worker::Owner;

/// The start time of `pid` in clock ticks since boot: field 22 of `/proc/<pid>/stat`,
/// counted after the `)` that ends the command name (which may itself hold spaces).
fn proc_start(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
}

/// Real liveness: `/proc` where there is one, else `kill -0` (pid only).
pub struct ProcLiveness;

impl Liveness for ProcLiveness {
    fn me(&self) -> Owner {
        let pid = std::process::id();
        Owner { pid, start: proc_start(pid) }
    }

    fn alive(&self, o: &Owner) -> bool {
        if Path::new("/proc/self").exists() {
            Path::new(&format!("/proc/{}", o.pid)).exists() && o.start.zip(proc_start(o.pid)).is_none_or(|(then, now)| then == now)
        } else {
            Command::new("kill").arg("-0").arg(o.pid.to_string()).stderr(Stdio::null()).status().map_or(true, |s| s.success())
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn real_liveness_knows_a_running_process_and_a_recycled_pid() {
        let me = ProcLiveness.me();
        assert!(me.start.is_some(), "the start time is read from /proc");
        assert!(ProcLiveness.alive(&me));
        assert!(ProcLiveness.alive(&Owner { pid: me.pid, start: None }), "pid alone is enough when no start time was saved");
        assert!(!ProcLiveness.alive(&Owner { pid: me.pid, start: me.start.map(|s| s + 1) }), "same pid, other start time: a different process");
        assert!(!ProcLiveness.alive(&Owner { pid: u32::MAX, start: Some(1) }), "no such process");
    }
}
