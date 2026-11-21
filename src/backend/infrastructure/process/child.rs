//! Multi-provider backend layer. A turn runs one CLI (`claude`, `codex`, ...) as a
//! child process that streams JSON lines on stdout. Everything the CLIs share, the
//! spawn, stdin hand-off, line reading, idle watchdog, stderr capture and error
//! mapping, lives once in [`run_turn`]; a [`Backend`] only says how to build its
//! command, how to read one output line, and how to classify its error text.

use std::collections::VecDeque;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::BufReader;
use tokio::process::Child;
use tokio::task::JoinHandle;

use super::output_pump::{Ended, Idle, Pump};
use super::process_group::GroupKill;
use super::stderr_tail::{collect_stderr, last_chars, STDERR_SHOWN};
use super::stdin_feed::StdinFeed;
use crate::backend::application::ports::{Approver, Backend, NoApprover, Steering};
use crate::backend::domain::error_kind::ErrorKind;
use crate::backend::domain::event::Event;
use crate::backend::domain::outcome_judge::{empty_after_denial, judge};
use crate::backend::domain::turn_error::TurnError;
use crate::backend::domain::turn_outcome::TurnOutcome;
use crate::backend::domain::turn_request::TurnRequest;

/// After the CLI has answered or died, how long to wait for it to be reaped.
const REAP_WAIT: Duration = Duration::from_secs(2);

/// Wait for the child to be reaped, killing it if it will not exit.
async fn reap(child: &mut Child) -> Option<std::process::ExitStatus> {
    match tokio::time::timeout(REAP_WAIT, child.wait()).await {
        Ok(status) => status.ok(),
        Err(_) => {
            let _ = child.start_kill();
            None
        }
    }
}

/// A stream that ended with no result: the stderr tail is the best explanation.
async fn explain_eof(child: &mut Child, stderr: JoinHandle<String>) -> String {
    let status = reap(child).await;
    let tail = tokio::time::timeout(REAP_WAIT, stderr).await.ok().and_then(Result::ok).unwrap_or_default();
    let tail = last_chars(&tail, STDERR_SHOWN);
    if !tail.is_empty() {
        return tail;
    }
    match status {
        Some(s) => format!("the CLI exited ({s}) without a result"),
        None => "the CLI ended without a result".into(),
    }
}

/// Run one turn through `backend` and return its answer or a classified error.
/// `on_event` sees `Tool`, `Text` and `Plan` events while the turn runs. The child
/// is killed if the future is dropped, and after `idle_timeout` of silence.
pub async fn run_turn(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
) -> Result<TurnOutcome, TurnError> {
    run_turn_with(backend, req, idle_timeout, on_event, &NoApprover).await
}

/// [`run_turn`] with someone to answer the tool approvals of an [`Backend::interactive`]
/// CLI. For any other backend `approver` is never asked.
pub async fn run_turn_with(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
    approver: &dyn Approver,
) -> Result<TurnOutcome, TurnError> {
    run_turn_steered(backend, req, idle_timeout, on_event, approver, None).await
}

/// [`run_turn_with`] that also takes messages the person types while the turn runs
/// (`steer`), for a backend that [`Backend::steerable`]. A message is written to the CLI as
/// it arrives and [`Event::Steered`] reports when the model has it. One that was not picked up
/// by the time the answer came is simply not reported: whoever sent it still has it.
pub async fn run_turn_steered(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
    approver: &dyn Approver,
    steer: Option<Steering<'_>>,
) -> Result<TurnOutcome, TurnError> {
    // A guard, not a call after the await: a cancelled turn drops this future, and the
    // backend's files must go then too.
    let _cleanup = CleanupOnDrop(backend);
    run_child(backend, req, idle_timeout, on_event, approver, steer).await
}

/// Runs [`Backend::cleanup`] when the turn ends or its future is dropped.
struct CleanupOnDrop<'a>(&'a dyn Backend);

impl Drop for CleanupOnDrop<'_> {
    fn drop(&mut self) {
        self.0.cleanup();
    }
}

async fn run_child(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
    approver: &dyn Approver,
    steer: Option<Steering<'_>>,
) -> Result<TurnOutcome, TurnError> {
    let mut cmd = backend.command(req);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    // Its own process group, so the whole tree can be killed (see [`GroupKill`]).
    #[cfg(unix)]
    cmd.process_group(0);
    let program = cmd.as_std().get_program().to_string_lossy().into_owned();
    let other = |m: String| TurnError::new(ErrorKind::Other, m);
    let mut child = cmd.spawn().map_err(|e| other(format!("failed to run `{program}` (is it installed?): {e}")))?;
    let mut group = GroupKill(if cfg!(unix) { child.id() } else { None });
    let stdout = child.stdout.take().ok_or_else(|| other(format!("`{program}` gave no stdout")))?;
    let mut feed = StdinFeed::start(child.stdin.take(), backend.stdin_payload(req), backend.interactive());
    let stderr = collect_stderr(child.stderr.take());

    let mut plan = None;
    let mut pump = Pump {
        backend,
        reader: BufReader::new(stdout),
        raw: Vec::new(),
        idle: idle_timeout,
        on_event,
        plan: &mut plan,
        approver,
        feed: &mut feed,
        finished: None,
        queue: VecDeque::new(),
        steer: steer.filter(|_| backend.steerable()),
        sent: VecDeque::new(),
        denied_unasked: Vec::new(),
    };
    let ended = pump.run().await;
    // Messages written to the CLI that it never took before the answer: they are still the
    // person's to run, so the CLI must not start on them now (it would read them from the pipe
    // and the next turn would run them a second time).
    let not_taken = !pump.sent.is_empty();
    let denied_unasked = std::mem::take(&mut pump.denied_unasked);
    drop(pump);
    let classified = |message: String| TurnError::new(backend.classify_error(&message), message);
    let result = match ended {
        Err(Idle) => {
            let _ = child.start_kill();
            let secs = idle_timeout.as_secs_f32();
            Err(TurnError::new(ErrorKind::Stalled, format!("`{program}` printed nothing for {secs}s; stopped it")))
        }
        Ok(Ended::Failed(message)) => {
            let _ = child.start_kill();
            Err(classified(message))
        }
        Ok(Ended::Finished(finish)) if not_taken => {
            // Dropping `group` kills everything the CLI started as well.
            let _ = child.start_kill();
            let _ = reap(&mut child).await;
            judge(finish, plan)
        }
        Ok(Ended::Finished(finish)) => {
            if reap(&mut child).await.is_some() {
                group.disarm();
            }
            judge(finish, plan)
        }
        Ok(Ended::Eof) => Err(classified(explain_eof(&mut child, stderr).await)),
    };
    result.map_err(|e| empty_after_denial(e, &denied_unasked))
}


#[cfg(test)]
mod tests;
