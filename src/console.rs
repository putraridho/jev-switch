//! The terminal output of one `jev-code` run: a [`Ui`] that prints a turn as it happens
//! (route, tool calls, notes, the answer), keeps a live status line under it while a model
//! works, and serialises every byte through ONE mutex so lines never interleave with the
//! status line (erase it, print, draw it again).
//!
//! All wording and styling comes from [`crate::uifmt`]; this module only decides what goes
//! where. Answers go to stdout, everything else (route, tools, notes, errors, the summary)
//! goes to stderr, so `jev-code -p ... > answer.txt` captures exactly the answer. The
//! writers are injected, so every behaviour is a unit test over byte buffers.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::backend::Event;
use crate::terminal::{self, Screen, StdScreen};
use crate::turn::{Failover, NoRouteInfo, Ui};
use crate::uifmt::{self, FileChange, RouteFacts, Style, SummaryFacts, Theme};

/// How often the status line is redrawn (the spinner frame changes every 100 ms).
pub const TICK: Duration = Duration::from_millis(100);

/// What the console needs to know about its surroundings.
#[derive(Debug, Clone)]
pub struct ConsoleConfig {
    /// Styling for what goes to stdout (the answer, command output).
    pub out_style: Style,
    /// Styling for what goes to stderr (progress).
    pub err_style: Style,
    /// The width to lay lines out for when the terminal cannot be asked (tests, pipes). A
    /// console on a real terminal asks it again on every draw, so a resize is followed.
    pub width: usize,
    /// Draw the live status line (stderr is a terminal that takes escape sequences).
    pub live: bool,
    /// The project folder, to show paths relative to it.
    pub project: PathBuf,
    /// Print assistant text to stdout as it arrives. Off for `-p`, where the caller prints
    /// the final answer itself.
    pub stream_answers: bool,
}

struct Pending {
    provider: String,
    model: String,
    kind: crate::backend::ErrorKind,
    discarded: bool,
}

struct Live {
    label: String,
    activity: Option<String>,
    started: Instant,
    drawn: bool,
}

struct Inner {
    out: Box<dyn Write + Send>,
    err: Box<dyn Write + Send>,
    cfg: ConsoleConfig,
    live: Option<Live>,
    cursor_hidden: bool,
    /// The terminal itself, when there is one to ask; `None` in tests and pipes.
    screen: Option<Box<dyn Screen>>,
    /// Typed keys are not echoed while the status line is up.
    echo_off: bool,
    /// Notices that arrived while the user was at the prompt, printed at the next input.
    deferred: Vec<String>,
    /// A failure whose "→ trying X" part is not known until the next route is chosen.
    pending: Option<Pending>,
    /// Set when no model could take the turn; the caller prints the full message, which
    /// needs facts the console does not have.
    no_route_task: Option<String>,
}

/// The terminal output. Cheap to clone; every clone writes through the same lock.
#[derive(Clone)]
pub struct Console {
    inner: Arc<Mutex<Inner>>,
}

impl Console {
    pub fn new(out: Box<dyn Write + Send>, err: Box<dyn Write + Send>, cfg: ConsoleConfig) -> Self {
        let inner = Inner { out, err, cfg, live: None, cursor_hidden: false, screen: None, echo_off: false, deferred: Vec::new(), pending: None, no_route_task: None };
        Self { inner: Arc::new(Mutex::new(inner)) }
    }

    /// A console on the process's real stdout and stderr.
    pub fn stdio(cfg: ConsoleConfig) -> Self {
        Self::new(Box::new(std::io::stdout()), Box::new(std::io::stderr()), cfg).with_screen(Box::new(StdScreen))
    }

    /// Follow this terminal: its live width replaces the configured one, and typed keys are
    /// not echoed over the status line.
    pub fn with_screen(self, screen: Box<dyn Screen>) -> Self {
        self.lock().screen = Some(screen);
        self
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn out_style(&self) -> Style {
        self.lock().cfg.out_style
    }

    pub fn err_style(&self) -> Style {
        self.lock().cfg.err_style
    }

    /// The columns lines are laid out for right now.
    pub fn width(&self) -> usize {
        self.lock().width()
    }

    /// Print text on stdout (command output), with a newline.
    pub fn out(&self, text: &str) {
        self.lock().emit_out(text);
    }

    /// Print text on stderr (messages), with a newline.
    pub fn err(&self, text: &str) {
        self.lock().emit_err(text);
    }

    /// A turn starts: the status line appears, saying the router is working.
    pub fn begin_turn(&self) {
        self.lock().begin();
    }

    /// The turn is over: erase the status line, show the cursor, and print a failure that
    /// never learned what came next.
    pub fn end_turn(&self) {
        self.lock().end();
    }

    /// Redraw the status line with the real elapsed time.
    pub fn tick(&self) {
        let mut g = self.lock();
        let elapsed = g.live.as_ref().map(|l| l.started.elapsed());
        if let Some(e) = elapsed {
            g.draw(e);
        }
    }

    /// Redraw the status line as if `elapsed` had passed (for tests).
    pub fn tick_at(&self, elapsed: Duration) {
        self.lock().draw(elapsed);
    }

    /// Say what the status line shows besides the model: "stopping..." after Ctrl-C.
    pub fn set_activity(&self, text: &str) {
        let mut g = self.lock();
        g.set_activity(text);
        g.refresh();
    }

    /// A line that is not part of any turn (a background check finished): printed at once
    /// while a turn is running, through the same lock as everything else, so it lands
    /// between lines and the status line is redrawn under it. At the prompt it is held until
    /// [`Console::flush_notices`], because printing there would land inside the line being typed.
    pub fn notify(&self, text: &str) {
        let mut g = self.lock();
        let line = format!("  {}", g.cfg.err_style.dim(text));
        if g.live.is_some() {
            g.emit_err(&line);
        } else {
            g.deferred.push(line);
        }
    }

    /// Print the notices that were held for the prompt. Call when input has just been read.
    pub fn flush_notices(&self) {
        let mut g = self.lock();
        for line in std::mem::take(&mut g.deferred) {
            g.emit_err(&line);
        }
    }

    /// A task that redraws the status line every [`TICK`] until dropped. Does nothing
    /// unless the console is live. Needs a running tokio runtime.
    pub fn ticker(&self) -> Ticker {
        if !self.lock().cfg.live {
            return Ticker(None);
        }
        let console = self.clone();
        Ticker(Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(TICK);
            loop {
                interval.tick().await;
                console.tick();
            }
        })))
    }

    /// If the last turn found no model able to take it, the task type; once.
    pub fn take_no_route(&self) -> Option<String> {
        self.lock().no_route_task.take()
    }

    /// The lines after a turn: the dim summary, the changed files, and how to review or
    /// revert them, then a blank line.
    pub fn turn_end(&self, summary: &SummaryFacts, files: &[FileChange]) {
        let mut g = self.lock();
        let (style, width) = (g.cfg.err_style, g.width());
        let mut lines = vec![format!("  {}", uifmt::turn_summary(summary, style))];
        if !files.is_empty() {
            lines.push(uifmt::changes_block(files, style, width));
            lines.push(format!("  {}", style.dim("/diff to review, /undo to revert")));
        }
        // The blank line after the last one keeps the next prompt apart from this turn.
        g.emit_err(&format!("{}\n\n", lines.join("\n")));
    }
}

/// Redraws the status line on a timer; stops when dropped.
pub struct Ticker(Option<tokio::task::JoinHandle<()>>);

impl Drop for Ticker {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

impl Inner {
    /// Columns right now: what the terminal reports, else the configured width.
    fn width(&self) -> usize {
        self.screen.as_ref().and_then(|s| s.columns()).unwrap_or(self.cfg.width).max(1)
    }

    /// Remove the status line from the screen (it is redrawn after the next print).
    fn erase(&mut self) {
        if let Some(l) = self.live.as_mut().filter(|l| l.drawn) {
            l.drawn = false;
            let _ = self.err.write_all(uifmt::clear_line().as_bytes());
        }
    }

    fn draw(&mut self, elapsed: Duration) {
        // The status line is redrawn in place (`\r`, erase the row), which only works while it
        // is ONE row: so it is cut by display width to one column short of the live width,
        // since a full row wraps on terminals that wrap at the last column.
        let budget = self.width().saturating_sub(1);
        let style = self.cfg.err_style;
        let Some(l) = self.live.as_mut() else { return };
        let line = uifmt::status_line(&l.label, elapsed, l.activity.as_deref(), budget, style);
        let mut frame = String::new();
        if !self.cursor_hidden {
            self.cursor_hidden = true;
            terminal::note_cursor_hidden(true);
            frame.push_str(uifmt::hide_cursor());
        }
        frame.push_str(uifmt::clear_line());
        frame.push_str(&line);
        l.drawn = true;
        let _ = self.err.write_all(frame.as_bytes());
        let _ = self.err.flush();
    }

    fn refresh(&mut self) {
        if let Some(elapsed) = self.live.as_ref().map(|l| l.started.elapsed()) {
            self.draw(elapsed);
        }
    }

    fn emit_err(&mut self, text: &str) {
        self.erase();
        let _ = self.err.write_all(text.as_bytes());
        if !text.ends_with('\n') {
            let _ = self.err.write_all(b"\n");
        }
        let _ = self.err.flush();
        self.refresh();
    }

    fn emit_out(&mut self, text: &str) {
        self.erase();
        // The two streams share one screen: finish what is pending on stderr first.
        let _ = self.err.flush();
        let _ = self.out.write_all(text.as_bytes());
        if !text.ends_with('\n') {
            let _ = self.out.write_all(b"\n");
        }
        let _ = self.out.flush();
        self.refresh();
    }

    fn begin(&mut self) {
        if !self.cfg.live {
            return;
        }
        self.live = Some(Live { label: "routing".into(), activity: None, started: Instant::now(), drawn: false });
        if let Some(screen) = &self.screen {
            screen.set_echo(false);
            self.echo_off = true;
        }
        self.refresh();
    }

    fn end(&mut self) {
        self.erase();
        self.live = None;
        if self.cursor_hidden {
            self.cursor_hidden = false;
            terminal::note_cursor_hidden(false);
            let _ = self.err.write_all(uifmt::show_cursor().as_bytes());
        }
        if std::mem::take(&mut self.echo_off) {
            if let Some(screen) = &self.screen {
                screen.set_echo(true);
            }
        }
        let _ = self.err.flush();
        if let Some(p) = self.pending.take() {
            let line = self.failover_line(&p, None);
            self.emit_err(&line);
        }
    }

    fn set_activity(&mut self, text: &str) {
        if let Some(l) = self.live.as_mut() {
            l.activity = Some(text.to_string());
        }
    }

    fn failover_line(&self, p: &Pending, to: Option<(&str, &str)>) -> String {
        format!("  {}", uifmt::failover_note(&p.provider, &p.model, &p.kind, to, p.discarded, self.cfg.err_style))
    }

    fn route(&mut self, f: &RouteFacts) {
        if let Some(p) = self.pending.take() {
            let line = self.failover_line(&p, Some((&f.provider, &f.model)));
            self.emit_err(&line);
        }
        if let Some(l) = self.live.as_mut() {
            l.label = if f.model.is_empty() { f.provider.clone() } else { format!("{} · {}", f.provider, f.model) };
            l.activity = None;
        }
        let line = format!("  {}", uifmt::route_line(f, self.cfg.err_style));
        self.emit_err(&line);
    }
}

/// A plain engine note ("[jev] unavailable ...") as a warning line: the bracketed tag
/// becomes a prefix (`jev: unavailable ...`).
pub fn note_line(line: &str, style: Style) -> String {
    let line = line.trim();
    let text = match line.strip_prefix('[').and_then(|r| r.split_once("] ")) {
        Some((tag, rest)) => format!("{tag}: {rest}"),
        None => line.to_string(),
    };
    format!("  {} {text}", style.yellow("⚠"))
}

impl Ui for Console {
    fn route(&mut self, line: &str) {
        let mut g = self.lock();
        let text = format!("  {}", g.cfg.err_style.dim(line));
        g.emit_err(&text);
    }

    fn route_facts(&mut self, facts: &RouteFacts) {
        self.lock().route(facts);
    }

    fn event(&mut self, e: &Event) {
        let mut g = self.lock();
        match e {
            Event::Tool(summary) => {
                let (style, width) = (g.cfg.err_style, g.width());
                let line = uifmt::tool_line(summary, &g.cfg.project, style, width);
                let activity = uifmt::relativize_paths(&uifmt::strip_ansi(summary), &g.cfg.project);
                g.set_activity(&activity);
                g.emit_err(&line);
            }
            // Permission hints start with ✗; they are diagnostics, so they stay on stderr.
            Event::Text(t) if t.starts_with('\u{2717}') => {
                let text = format!("  {}", g.cfg.err_style.yellow(&uifmt::strip_ansi(t)));
                g.emit_err(&text);
            }
            Event::Text(t) if g.cfg.stream_answers => {
                let text = uifmt::answer_text(t, g.cfg.out_style, g.width());
                g.emit_out(&format!("{text}\n"));
            }
            _ => {}
        }
    }

    fn note(&mut self, line: &str) {
        let mut g = self.lock();
        let style = g.cfg.err_style;
        for l in line.lines().filter(|l| !l.trim().is_empty()) {
            let text = note_line(l, style);
            g.emit_err(&text);
        }
    }

    fn failover(&mut self, f: &Failover) {
        let mut g = self.lock();
        if f.will_retry {
            g.pending = Some(Pending {
                provider: f.provider.to_string(),
                model: f.model.to_string(),
                kind: f.kind.clone(),
                discarded: f.discarded,
            });
            return;
        }
        let style = g.cfg.err_style;
        let detail = format!("{} {}: {}", f.provider, f.model, f.message.trim());
        let block = uifmt::error_block(f.kind, &detail, &uifmt::next_steps_for(f.kind, f.provider.as_str()), style);
        g.emit_err(&block);
    }

    fn no_route(&mut self, info: &NoRouteInfo, _text: &str) {
        let mut g = self.lock();
        match info {
            NoRouteInfo::Nothing { task } => g.no_route_task = Some((*task).to_string()),
            NoRouteInfo::Pin { what } => {
                let steps = ["Let the router choose again: `/model auto`".to_string(), "See what can run now: `/providers`".to_string()];
                let block = uifmt::problem_block("Your /model pin can't run right now", &[one_sentence(what)], &steps, g.cfg.err_style);
                g.emit_err(&block);
            }
        }
    }

    /// The `■ stopped` summary says it; no extra line.
    fn cancelled(&mut self, _text: &str) {}
}

/// `what` without a trailing period, so a detail line reads cleanly.
fn one_sentence(what: &str) -> String {
    what.trim().trim_end_matches('.').to_string()
}

/// The provider's name in its colour, for lines built outside this module.
pub fn provider_name(provider: &str, style: Style) -> String {
    Theme::new(style).name(provider)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    /// A writer whose bytes can be read back.
    #[derive(Clone, Default)]
    pub struct Buf(pub Arc<Mutex<Vec<u8>>>);

    impl Buf {
        pub fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
        pub fn clear(&self) {
            self.0.lock().unwrap().clear();
        }
    }

    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::testing::Buf;
    use super::*;
    use crate::backend::{ErrorKind, ProviderId};

    fn console(live: bool, style: Style, stream_answers: bool) -> (Console, Buf, Buf) {
        let (out, err) = (Buf::default(), Buf::default());
        let cfg = ConsoleConfig { out_style: style, err_style: style, width: 60, live, project: PathBuf::from("/proj"), stream_answers };
        (Console::new(Box::new(out.clone()), Box::new(err.clone()), cfg), out, err)
    }

    fn facts(provider: &str, model: &str) -> RouteFacts {
        RouteFacts { provider: provider.into(), model: model.into(), task: "explain".into(), reason: "task: explain (0.9) → x; first turn".into(), ..RouteFacts::default() }
    }

    fn quota() -> ErrorKind {
        ErrorKind::Quota { resets_at: Some("5pm".into()) }
    }

    #[test]
    fn a_plain_console_prints_route_tools_and_answers_without_any_escape() {
        let (mut c, out, err) = console(false, Style::off(), true);
        c.route_facts(&facts("claude", "sonnet-5-5"));
        c.event(&Event::Tool("Edit /proj/src/a.rs".into()));
        c.event(&Event::Text("✗ denied Bash".into()));
        c.event(&Event::Text("the answer".into()));
        let e = err.text();
        assert!(e.contains("  → claude · sonnet-5-5   explain · first turn"), "{e}");
        assert!(e.contains("  ⏺ Edit src/a.rs"), "{e}");
        assert!(e.contains("  ✗ denied Bash"), "{e}");
        assert_eq!(out.text(), "the answer\n");
        assert!(!e.contains('\x1b') && !out.text().contains('\x1b'));
    }

    #[test]
    fn answers_do_not_stream_in_print_mode_but_hints_still_reach_stderr() {
        let (mut c, out, err) = console(false, Style::off(), false);
        c.event(&Event::Text("the answer".into()));
        c.event(&Event::Text("✗ denied Bash".into()));
        assert_eq!(out.text(), "");
        assert!(err.text().contains("denied Bash"));
    }

    #[test]
    fn the_answer_is_laid_out_for_the_terminal_when_styled() {
        let (mut c, out, _) = console(false, Style::on(), true);
        c.event(&Event::Text("# Title\n\nuse `cargo test`".into()));
        let o = out.text();
        assert!(o.contains("\x1b[1mTitle") && o.contains("\x1b[36m"), "{o:?}");
    }

    #[test]
    fn a_live_console_draws_the_status_line_and_keeps_lines_from_interleaving_with_it() {
        let (mut c, _, err) = console(true, Style::off(), true);
        c.begin_turn();
        let first = err.text();
        assert!(first.starts_with(uifmt::hide_cursor()), "{first:?}");
        assert!(first.contains("\r\x1b[2K") && first.contains("routing"), "{first:?}");
        err.clear();

        c.route_facts(&facts("codex", "gpt-6"));
        c.tick_at(Duration::from_millis(2500));
        c.event(&Event::Tool("Bash cargo test".into()));
        let e = err.text();
        // Every printed line is preceded by an erase, and followed by a redraw of the status.
        let erase = uifmt::clear_line();
        let route_at = e.find("→ codex · gpt-6").expect("route line");
        assert!(e[..route_at].ends_with(&format!("{erase}  ")), "the status line is erased before the route line: {e:?}");
        let tool_at = e.find("⏺ Bash cargo test").expect("tool line");
        let status_after_tool = &e[tool_at..];
        assert!(status_after_tool.contains("codex · gpt-6") && status_after_tool.contains("Bash cargo test"), "{e:?}");
        assert!(e.contains("2s"), "the tick shows the elapsed time: {e:?}");
        c.end_turn();
    }

    #[test]
    fn ending_a_turn_erases_the_status_line_and_shows_the_cursor() {
        let (c, _, err) = console(true, Style::off(), true);
        c.begin_turn();
        err.clear();
        c.end_turn();
        assert_eq!(err.text(), format!("{}{}", uifmt::clear_line(), uifmt::show_cursor()));
        // After it, ticking draws nothing.
        err.clear();
        c.tick();
        assert_eq!(err.text(), "");
    }

    #[test]
    fn a_console_that_is_not_live_never_draws_anything() {
        let (c, _, err) = console(false, Style::off(), true);
        c.begin_turn();
        c.tick();
        c.end_turn();
        assert_eq!(err.text(), "");
    }

    #[test]
    fn a_failover_waits_for_the_next_route_and_names_it() {
        let (mut c, _, err) = console(false, Style::off(), true);
        c.route_facts(&facts("claude", "opus"));
        err.clear();
        let kind = quota();
        c.failover(&Failover {
            provider: ProviderId::Claude,
            model: "opus",
            kind: &kind,
            message: "limit",
            discarded: false,
            will_retry: true,
            text: "x",
        });
        assert_eq!(err.text(), "", "nothing yet: the next model is not known");
        c.route_facts(&facts("codex", "gpt-6"));
        let e = err.text();
        let note_at = e.find("⚠ claude hit its limit (resets 5pm) → trying codex · gpt-6").expect(&e);
        assert!(note_at < e.find("→ codex · gpt-6   ").unwrap(), "the note comes before the route line: {e}");
    }

    #[test]
    fn a_failover_that_ends_the_turn_is_flushed_without_a_next_model() {
        let (mut c, _, err) = console(false, Style::off(), true);
        let kind = quota();
        c.failover(&Failover {
            provider: ProviderId::Claude,
            model: "opus",
            kind: &kind,
            message: "limit",
            discarded: true,
            will_retry: true,
            text: "x",
        });
        c.end_turn();
        let e = err.text();
        assert!(e.contains("⚠ claude hit its limit (resets 5pm) (partial output discarded)") && !e.contains("trying"), "{e}");
    }

    #[test]
    fn a_final_failure_is_an_error_block_with_next_steps_and_no_debug_output() {
        let (mut c, _, err) = console(false, Style::off(), true);
        let kind = ErrorKind::Auth;
        c.failover(&Failover {
            provider: ProviderId::Codex,
            model: "gpt-6",
            kind: &kind,
            message: "401 Unauthorized",
            discarded: false,
            will_retry: false,
            text: "[error] codex gpt-6 failed: 401 Unauthorized",
        });
        let e = err.text();
        assert!(e.starts_with("✗ Login needed\n  codex gpt-6: 401 Unauthorized\n  Try:\n    • Run `codex login`"), "{e}");
        assert!(!e.contains("Auth") && !e.contains("{"), "no Debug output: {e}");
    }

    #[test]
    fn no_route_for_a_task_is_left_for_the_caller_and_a_bad_pin_is_explained() {
        let (mut c, _, err) = console(false, Style::off(), true);
        c.no_route(&NoRouteInfo::Nothing { task: "security" }, "text");
        assert_eq!(err.text(), "");
        assert_eq!(c.take_no_route().as_deref(), Some("security"));
        assert_eq!(c.take_no_route(), None, "taken once");

        c.no_route(&NoRouteInfo::Pin { what: "codex:gpt-6 is not usable now." }, "text");
        let e = err.text();
        assert!(e.contains("✗ Your /model pin can't run right now") && e.contains("codex:gpt-6 is not usable now\n"), "{e}");
        assert!(e.contains("`/model auto`") && e.contains("`/providers`"), "{e}");
    }

    #[test]
    fn a_cancelled_turn_prints_nothing_of_its_own() {
        let (mut c, out, err) = console(false, Style::off(), true);
        c.cancelled("[cancelled] stopped by the user");
        assert_eq!((out.text(), err.text()), (String::new(), String::new()));
    }

    #[test]
    fn plain_notes_become_warning_lines() {
        assert_eq!(note_line("[jev] unavailable (timeout); routing without its judgment", Style::off()), "  ⚠ jev: unavailable (timeout); routing without its judgment");
        assert_eq!(note_line("something else", Style::off()), "  ⚠ something else");
        let (mut c, _, err) = console(false, Style::off(), true);
        c.note("[orchestrate] could not write x\n\nsecond");
        assert_eq!(err.text(), "  ⚠ orchestrate: could not write x\n  ⚠ second\n");
    }

    #[test]
    fn the_end_of_a_turn_shows_the_summary_files_and_how_to_review_them() {
        let (c, _, err) = console(false, Style::off(), true);
        let summary = SummaryFacts { ok: true, provider: "claude".into(), model: "sonnet".into(), elapsed_ms: 14_000, files_changed: Some((1, 3, 1)), ..SummaryFacts::default() };
        let files = [FileChange { status: 'M', path: "src/a.rs".into(), added: 3, removed: 1 }];
        c.turn_end(&summary, &files);
        let e = err.text();
        assert!(e.starts_with("  ✓ claude · sonnet · 14s"), "{e}");
        assert!(e.contains("  M src/a.rs") && e.contains("+3 −1"), "{e}");
        assert!(e.contains("  /diff to review, /undo to revert\n"), "{e}");
        assert!(e.ends_with("\n\n"), "a blank line separates turns: {e:?}");

        err.clear();
        c.turn_end(&SummaryFacts { ok: true, cancelled: true, elapsed_ms: 3000, ..SummaryFacts::default() }, &[]);
        let e = err.text();
        assert!(e.contains("■ stopped · 3s") && !e.contains("/diff"), "{e}");
    }

    /// A terminal whose width the test changes, and that remembers echo switches.
    #[derive(Clone)]
    struct FakeScreen {
        cols: Arc<AtomicUsize>,
        echo: Arc<Mutex<Vec<bool>>>,
    }

    impl FakeScreen {
        fn new(cols: usize) -> Self {
            Self { cols: Arc::new(AtomicUsize::new(cols)), echo: Arc::default() }
        }
    }

    impl Screen for FakeScreen {
        fn columns(&self) -> Option<usize> {
            Some(self.cols.load(Ordering::SeqCst))
        }
        fn set_echo(&self, on: bool) {
            self.echo.lock().unwrap().push(on);
        }
    }

    fn on_screen(cols: usize) -> (Console, Buf, FakeScreen) {
        let (c, _, err) = console(true, Style::off(), true);
        let screen = FakeScreen::new(cols);
        (c.with_screen(Box::new(screen.clone())), err, screen)
    }

    /// The status frames drawn since the last clear: each is what follows an erase-row.
    fn frames(drawn: &str) -> Vec<String> {
        drawn.split(uifmt::clear_line()).skip(1).map(|f| f.replace(uifmt::hide_cursor(), "")).collect()
    }

    const LONG_COMMAND: &str = "Bash ls -R | head -100 && echo --- && ls | wc -l && cat log --oneline --decorate --graph --all | head -50 && echo done and more and more text";

    #[test]
    fn the_status_line_follows_the_live_terminal_width_not_the_width_at_startup() {
        let (mut c, err, screen) = on_screen(120);
        c.begin_turn();
        c.route_facts(&facts("claude", "claude-opus-5-5"));
        c.event(&Event::Tool(LONG_COMMAND.into()));
        for cols in [120, 70, 33, 61, 200, 5, 1] {
            screen.cols.store(cols, Ordering::SeqCst);
            err.clear();
            c.tick_at(Duration::from_secs(9));
            let drawn = frames(&err.text());
            assert_eq!(drawn.len(), 1, "one frame per tick at {cols} columns");
            let width = uifmt::display_width(&drawn[0]);
            assert!(width < cols.max(2), "a {width}-column frame reaches the last of {cols} columns: {:?}", drawn[0]);
            assert!(!drawn[0].contains('\n'), "one row: {:?}", drawn[0]);
        }
        assert_eq!(c.width(), 1);
    }

    #[test]
    fn a_status_line_never_carries_control_characters_from_a_tool_summary() {
        let (mut c, err, _screen) = on_screen(80);
        c.begin_turn();
        for hostile in ["Bash echo \x1b[31mred\x1b[0m\r\nnext\x07 bell \x1b]0;title\x07 end \x1b", "Bash tab\there\x08\x7f\u{9b}31m", "\x1b"] {
            err.clear();
            c.event(&Event::Tool(hostile.into()));
            let all = err.text();
            // Erase, the tool line (only its final newline is a control character), erase, the status frame: two parts after each erase.
            let parts = frames(&all);
            assert_eq!(parts.len(), 2, "{all:?}");
            assert!(!parts[0].trim_end_matches('\n').chars().any(char::is_control), "{hostile:?} leaked into the tool line {:?}", parts[0]);
            assert!(!parts[1].chars().any(char::is_control), "{hostile:?} leaked into the status line {:?}", parts[1]);
        }
    }

    #[test]
    fn keys_typed_during_a_turn_are_not_echoed_over_the_status_line() {
        let (c, _, screen) = on_screen(80);
        c.begin_turn();
        c.end_turn();
        assert_eq!(*screen.echo.lock().unwrap(), [false, true]);

        // A console that draws nothing does not touch the terminal's modes either.
        let (plain, _, _) = console(false, Style::off(), true);
        let quiet = FakeScreen::new(80);
        let plain = plain.with_screen(Box::new(quiet.clone()));
        plain.begin_turn();
        plain.end_turn();
        assert!(quiet.echo.lock().unwrap().is_empty());
    }

    #[test]
    fn a_notice_waits_for_the_prompt_unless_a_turn_is_running() {
        let (c, _, err) = console(true, Style::off(), true);
        c.notify("codex is ready now");
        assert_eq!(err.text(), "", "at the prompt it must not land inside the line being typed");
        c.flush_notices();
        assert_eq!(err.text(), "  codex is ready now\n");
        c.flush_notices();
        assert_eq!(err.text(), "  codex is ready now\n", "printed once");

        err.clear();
        c.begin_turn();
        err.clear();
        c.notify("codex is ready now");
        let e = err.text();
        let at = e.find("  codex is ready now\n").expect(&e);
        assert!(e[..at].ends_with(uifmt::clear_line()), "the status line is erased first: {e:?}");
        assert!(e[at..].contains("routing"), "and drawn again under it: {e:?}");
    }

    #[test]
    fn output_from_several_threads_never_splits_a_line() {
        let (c, _, err) = console(false, Style::off(), true);
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let c = c.clone();
                std::thread::spawn(move || {
                    for j in 0..50 {
                        c.err(&format!("line-{i}-{j}-end"));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let e = err.text();
        assert_eq!(e.lines().count(), 400);
        assert!(e.lines().all(|l| l.starts_with("line-") && l.ends_with("-end")), "{e}");
    }
}
