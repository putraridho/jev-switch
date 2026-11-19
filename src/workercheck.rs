//! Runs a finished worker's tests in its own worktree, so jev can merge the work without the
//! planner reading the diff: work that passes the project's own tests and applies cleanly
//! lands on its own, and anything else goes back to the planner for review.
//!
//! The command comes from [`crate::projectkind`] (`cargo test`, `npm test`, `go test ./...`,
//! pytest); a folder that is several kinds of project runs every one of them. It runs with
//! `sh -c` in the worktree, stdin closed, `CI=true` so test runners do not wait in watch mode,
//! and in its own process group so a timeout or a cancel kills the test binaries the command
//! started, not only the shell.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How long a worker's tests may run before the work goes to review instead.
pub const TEST_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// How much of the end of a failed run's output goes back to the planner.
pub const OUTPUT_TAIL_CHARS: usize = 3_000;
/// How often a running check looks at the clock and the cancel flag.
const POLL: Duration = Duration::from_millis(200);

/// How one test run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckRun {
    Passed,
    /// It exited non-zero or was killed by a signal; the end of its output.
    Failed { output: String },
    /// It ran past the timeout and was killed.
    TimedOut { after: Duration },
    /// The delegation was cancelled while it ran; it was killed.
    Stopped,
    /// It could not be started at all.
    CouldNotStart { error: String },
}

/// Finds and runs a project's tests. Real: [`ProjectChecks`]; tests use a fake.
pub trait Checks: Send + Sync {
    /// The command that tests the project at `dir`, or `None` when there is none.
    fn test_command(&self, dir: &Path) -> Option<String>;
    /// Run `command` in `dir` until it ends, the timeout passes, or `stop()` says to give up.
    /// Blocks: the caller runs it off the async threads.
    fn run(&self, dir: &Path, command: &str, stop: &dyn Fn() -> bool) -> CheckRun;
}

/// The project's own test commands, run for real.
pub struct ProjectChecks {
    pub timeout: Duration,
}

impl Default for ProjectChecks {
    fn default() -> Self {
        Self { timeout: TEST_TIMEOUT }
    }
}

/// Every test command `dir` has, joined so all must pass, or `None` when it has none.
pub fn detected_test_command(dir: &Path) -> Option<String> {
    let commands: Vec<String> = crate::projectkind::detect(dir).into_iter().filter_map(|p| p.test).collect();
    (!commands.is_empty()).then(|| commands.join(" && "))
}

impl Checks for ProjectChecks {
    fn test_command(&self, dir: &Path) -> Option<String> {
        detected_test_command(dir)
    }

    fn run(&self, dir: &Path, command: &str, stop: &dyn Fn() -> bool) -> CheckRun {
        run_with_timeout(dir, command, self.timeout, stop)
    }
}

/// A file for the run's output outside the worktree: inside, it would show up in the diff.
fn output_file() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!("jev-check-{}-{}.log", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)))
}

fn spawn(dir: &Path, command: &str, out: &File) -> std::io::Result<Child> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command).current_dir(dir).env("CI", "true").stdin(Stdio::null()).stdout(out.try_clone()?).stderr(out.try_clone()?);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }
    cmd.spawn()
}

/// Kill the whole process group the shell leads, then reap the shell.
fn kill_group(child: &mut Child) {
    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(child.id()) {
            // SAFETY: kill(2) with a negative pid signals that process group; no memory is involved.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The last `max` characters of the file, with a note when the start was cut.
fn tail(file: &mut File, max: usize) -> String {
    // Four bytes per character at most, so this many bytes always holds `max` characters.
    let keep = (max * 4) as u64;
    let len = file.metadata().map_or(0, |m| m.len());
    let from = len.saturating_sub(keep);
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(from)).is_err() || file.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let chars: Vec<char> = text.chars().collect();
    let cut = chars.len().saturating_sub(max);
    let kept: String = chars[cut..].iter().collect();
    if from > 0 || cut > 0 {
        format!("[earlier output not shown]\n{}", kept.trim_end())
    } else {
        kept.trim_end().to_string()
    }
}

fn run_with_timeout(dir: &Path, command: &str, timeout: Duration, stop: &dyn Fn() -> bool) -> CheckRun {
    let path = output_file();
    let result = (|| {
        let mut out = File::options().create(true).truncate(true).read(true).write(true).open(&path).map_err(|e| e.to_string())?;
        let mut child = spawn(dir, command, &out).map_err(|e| e.to_string())?;
        let started = Instant::now();
        let ended = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(e) => {
                    kill_group(&mut child);
                    return Err(e.to_string());
                }
            }
            if stop() {
                kill_group(&mut child);
                return Ok(CheckRun::Stopped);
            }
            if started.elapsed() >= timeout {
                kill_group(&mut child);
                break None;
            }
            std::thread::sleep(POLL);
        };
        // Once the shell has exited and been reaped its pid may be reused, so its group is
        // never signalled after that; anything it left in the background is not waited for.
        Ok(match ended {
            None => CheckRun::TimedOut { after: timeout },
            Some(status) if status.success() => CheckRun::Passed,
            Some(_) => CheckRun::Failed { output: tail(&mut out, OUTPUT_TAIL_CHARS) },
        })
    })();
    let _ = std::fs::remove_file(&path);
    result.unwrap_or_else(|error| CheckRun::CouldNotStart { error })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projectkind::tempdir::Dir;

    fn never() -> bool {
        false
    }

    #[test]
    fn a_passing_command_passes_and_a_failing_one_returns_its_output() {
        let d = Dir::new();
        assert_eq!(run_with_timeout(d.path(), "true", Duration::from_secs(10), &never), CheckRun::Passed);
        let failed = run_with_timeout(d.path(), "echo first; echo boom >&2; exit 3", Duration::from_secs(10), &never);
        assert_eq!(failed, CheckRun::Failed { output: "first\nboom".into() }, "stdout and stderr both, in order");
    }

    #[test]
    fn it_runs_in_the_given_folder_with_ci_set_and_leaves_nothing_there() {
        let d = Dir::new();
        d.write("marker", "");
        assert_eq!(run_with_timeout(d.path(), "test -f marker && test \"$CI\" = true", Duration::from_secs(10), &never), CheckRun::Passed);
        let names: Vec<_> = std::fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["marker"], "the output file lives outside the worktree");
    }

    #[test]
    fn a_run_past_the_timeout_is_killed_with_its_children() {
        let d = Dir::new();
        let started = Instant::now();
        // The child writes a file after a second if it survives the kill.
        let run = run_with_timeout(d.path(), "(sleep 1; touch survived) & sleep 30", Duration::from_millis(300), &never);
        assert_eq!(run, CheckRun::TimedOut { after: Duration::from_millis(300) });
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1300));
        assert!(!d.path().join("survived").exists(), "the whole process group was killed");
    }

    #[test]
    fn a_cancel_stops_the_run() {
        let d = Dir::new();
        let started = Instant::now();
        assert_eq!(run_with_timeout(d.path(), "sleep 30", Duration::from_secs(60), &|| started.elapsed() > Duration::from_millis(300)), CheckRun::Stopped);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_long_output_keeps_only_its_end() {
        let d = Dir::new();
        let run = run_with_timeout(d.path(), "i=0; while [ $i -lt 2000 ]; do echo line$i; i=$((i+1)); done; exit 1", Duration::from_secs(20), &never);
        let CheckRun::Failed { output } = run else { panic!("{run:?}") };
        assert!(output.starts_with("[earlier output not shown]") && output.ends_with("line1999"), "{output}");
        assert!(output.chars().count() <= OUTPUT_TAIL_CHARS + 40);
    }

    #[test]
    fn every_project_kind_in_the_folder_is_tested() {
        let d = Dir::new();
        assert_eq!(detected_test_command(d.path()), None);
        d.write("Cargo.toml", "");
        d.write("go.mod", "");
        assert_eq!(detected_test_command(d.path()).as_deref(), Some("cargo test && go test ./..."));
    }
}
