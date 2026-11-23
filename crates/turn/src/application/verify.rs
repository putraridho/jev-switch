//! Checking a finished turn's work with a command in the project folder.

use std::path::Path;
use std::sync::Arc;

use jev_orchestration::application::ports::{CheckRun, Checks};
use crate::application::cancel::Cancel;
use crate::domain::verification::Verification;

/// How a finished run counts as a verdict on the work.
pub fn verification_of(run: CheckRun) -> Verification {
    match run {
        CheckRun::Passed => Verification::Passed,
        CheckRun::Failed { output } => Verification::Failed { output },
        CheckRun::TimedOut { after } => Verification::Inconclusive { why: format!("the check ran past {}s and was stopped", after.as_secs()) },
        CheckRun::Stopped => Verification::Inconclusive { why: "stopped before the check ended".to_string() },
        CheckRun::CouldNotStart { error } => Verification::Inconclusive { why: format!("the check could not start: {error}") },
    }
}

/// Run `command` in the project at `dir` off the async threads; a pass and a failure are
/// verdicts, anything else (timeout, stop, could not start) is not.
pub async fn verify(checks: Arc<dyn Checks>, dir: &Path, command: &str, cancel: &Cancel) -> Verification {
    let (dir, command, stop) = (dir.to_path_buf(), command.to_string(), cancel.clone());
    match tokio::task::spawn_blocking(move || checks.run(&dir, &command, &|| stop.is_cancelled())).await {
        Ok(run) => verification_of(run),
        Err(e) => Verification::Inconclusive { why: format!("the check did not finish: {e}") },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct Fake {
        run: CheckRun,
    }

    impl Checks for Fake {
        fn test_command(&self, _dir: &Path) -> Option<String> {
            None
        }
        fn run(&self, _dir: &Path, _command: &str, _stop: &dyn Fn() -> bool) -> CheckRun {
            self.run.clone()
        }
    }

    async fn run(run: CheckRun) -> Verification {
        verify(Arc::new(Fake { run }), Path::new("."), "t", &Cancel::new()).await
    }

    #[tokio::test]
    async fn a_pass_and_a_failure_are_verdicts() {
        assert_eq!(run(CheckRun::Passed).await, Verification::Passed);
        assert_eq!(run(CheckRun::Failed { output: "1 failed".into() }).await, Verification::Failed { output: "1 failed".into() });
    }

    #[tokio::test]
    async fn no_result_is_not_a_verdict() {
        assert_eq!(run(CheckRun::TimedOut { after: Duration::from_secs(900) }).await.summary(), "[verify] not checked: the check ran past 900s and was stopped");
        assert_eq!(run(CheckRun::Stopped).await.tag(), "inconclusive");
        assert_eq!(run(CheckRun::CouldNotStart { error: "sh".into() }).await.tag(), "inconclusive");
    }
}
