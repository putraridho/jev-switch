//! Checking a finished turn's work with the project's own tests.

use std::path::Path;
use std::sync::Arc;

use jev_orchestration::application::ports::{CheckRun, Checks};
use crate::application::cancel::Cancel;
use crate::domain::acceptance::Acceptance;

/// How a finished run counts as a verdict on the work.
pub fn acceptance_of(run: CheckRun) -> Acceptance {
    match run {
        CheckRun::Passed => Acceptance::Passed,
        CheckRun::Failed { output } => Acceptance::Failed { output },
        CheckRun::TimedOut { after } => Acceptance::Inconclusive { why: format!("the tests ran past {}s and were stopped", after.as_secs()) },
        CheckRun::Stopped => Acceptance::Inconclusive { why: "stopped before the tests ended".to_string() },
        CheckRun::CouldNotStart { error } => Acceptance::Inconclusive { why: format!("the tests could not start: {error}") },
    }
}

/// Run the tests of the project at `dir` off the async threads. `None` when `dir` has no
/// test command, so nothing is claimed either way.
pub async fn verify(checks: Arc<dyn Checks>, dir: &Path, cancel: &Cancel) -> Acceptance {
    let Some(command) = checks.test_command(dir) else {
        return Acceptance::Inconclusive { why: "no test command found for this folder".to_string() };
    };
    let (dir, stop) = (dir.to_path_buf(), cancel.clone());
    match tokio::task::spawn_blocking(move || checks.run(&dir, &command, &|| stop.is_cancelled())).await {
        Ok(run) => acceptance_of(run),
        Err(e) => Acceptance::Inconclusive { why: format!("the test run did not finish: {e}") },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct Fake {
        command: Option<&'static str>,
        run: CheckRun,
    }

    impl Checks for Fake {
        fn test_command(&self, _dir: &Path) -> Option<String> {
            self.command.map(str::to_string)
        }
        fn run(&self, _dir: &Path, _command: &str, _stop: &dyn Fn() -> bool) -> CheckRun {
            self.run.clone()
        }
    }

    async fn run(command: Option<&'static str>, run: CheckRun) -> Acceptance {
        verify(Arc::new(Fake { command, run }), Path::new("."), &Cancel::new()).await
    }

    #[tokio::test]
    async fn a_pass_and_a_failure_are_verdicts() {
        assert_eq!(run(Some("t"), CheckRun::Passed).await, Acceptance::Passed);
        assert_eq!(run(Some("t"), CheckRun::Failed { output: "1 failed".into() }).await, Acceptance::Failed { output: "1 failed".into() });
    }

    #[tokio::test]
    async fn no_command_or_no_result_is_not_a_verdict() {
        assert_eq!(run(None, CheckRun::Passed).await.tag(), "inconclusive");
        assert_eq!(run(Some("t"), CheckRun::TimedOut { after: Duration::from_secs(900) }).await.summary(), "[verify] not checked: the tests ran past 900s and were stopped");
        assert_eq!(run(Some("t"), CheckRun::Stopped).await.tag(), "inconclusive");
        assert_eq!(run(Some("t"), CheckRun::CouldNotStart { error: "sh".into() }).await.tag(), "inconclusive");
    }
}
