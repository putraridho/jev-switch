//! `-p --output-format json`: the same one turn as [`Repl::one_shot`], with the result as a
//! JSON object on stdout instead of the answer text. (Its test is in `repl.rs`, with the rig.)

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::tui::repl::session::{run_turn, Interrupt, Repl, TurnEnd};
use crate::tui::app::headless_output::{JsonResult, OutputFormat};
use crate::turn::domain::schedule::Repeat;
use crate::tui::views::status_facts as views;

/// Waits for a delay; the real one sleeps, a test's returns at once.
pub type Pause = Box<dyn FnMut(Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// The real pause: sleep.
pub fn sleeper() -> Pause {
    Box::new(|d| Box::pin(tokio::time::sleep(d)))
}

impl Repl {
    /// `-p ... --every <schedule>`: the same prompt again and again, each run a turn of this
    /// session, until `repeat.times` runs are done or Ctrl-C (also while waiting). The exit
    /// code is the last run's; a run that failed does not stop the loop, as a watcher must
    /// outlive one bad run.
    pub async fn repeat_shot(&mut self, prompt: &str, format: OutputFormat, repeat: &Repeat, interrupt: &mut dyn Interrupt, pause: &mut Pause) -> i32 {
        let style = self.console.err_style();
        let mut run = 0u32;
        loop {
            run += 1;
            let code = self.one_shot_as(prompt, format, interrupt).await;
            if code == 130 || repeat.times.is_some_and(|t| run >= t) {
                return code;
            }
            let Some(delay) = repeat.schedule.delay_from(crate::shared::infrastructure::jsonl_log::now_ms() / 1000) else {
                self.console.err(&format!("  {} the schedule never fires again", style.yellow("⚠")));
                return code;
            };
            self.console.err(&style.dim(&format!("  run {run} {}; next in {}s (Ctrl-C stops)", if code == 0 { "answered" } else { "failed" }, delay.as_secs())));
            interrupt.drain();
            let stopped = tokio::select! {
                () = (pause)(delay) => false,
                () = interrupt.wait() => true,
            };
            if stopped {
                return 130;
            }
        }
    }

    /// One turn printed in `format`. The process exit code, as for [`Repl::one_shot`].
    pub async fn one_shot_as(&mut self, prompt: &str, format: OutputFormat, interrupt: &mut dyn Interrupt) -> i32 {
        if format == OutputFormat::Text {
            return self.one_shot(prompt, interrupt).await;
        }
        let prompt = self.take_attachments(prompt);
        match run_turn(&mut self.engine, &self.console, &prompt, interrupt).await {
            TurnEnd::Aborted => 130,
            TurnEnd::Done(report, elapsed) => {
                self.console.out(&JsonResult::of(&report, elapsed.as_millis() as u64).line());
                self.finish(&report, elapsed);
                views::exit_code(report.status)
            }
        }
    }
}
