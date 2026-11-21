use super::*;

impl Console {
    pub fn new(out: Box<dyn Write + Send>, err: Box<dyn Write + Send>, cfg: ConsoleConfig) -> Self {
        let inner = Inner {
            out,
            err,
            cfg,
            live: None,
            asking: None,
            cursor_hidden: false,
            screen: None,
            echo_before: None,
            deferred: Vec::new(),
            pending: None,
            no_route_task: None,
            held: None,
            last_route: None,
            draft: String::new(),
            type_ahead: false,
            queued: std::collections::VecDeque::new(),
            front: None,
            pending_text: None,
            live_answer: String::new(),
            tools_waiting: std::collections::VecDeque::new(),
            run: Vec::new(),
            group: uifmt::ToolGroup::default(),
            fresh_turn: true,
            steer: None,
            withdrawn: Vec::new(),
            show_thinking: show_thinking_from_env(),
        };
        Self {
            inner: Arc::new(Mutex::new(inner)),
            keys: None,
            paging: Arc::new(tokio::sync::watch::Sender::new(false)),
            pager_turn: Arc::new(tokio::sync::Mutex::new(())),
            type_ahead: false,
        }
    }

    /// Let the person type during a turn: Enter queues a message, Esc stops the turn.
    pub fn with_type_ahead(mut self) -> Self {
        self.type_ahead = true;
        self.lock().type_ahead = true;
        self
    }

    /// Hand the screen to an interactive front end ([`crate::tui`]): from now on lines,
    /// the turn's status and approvals go to it as messages, and it reads the keys.
    pub fn with_front(self, front: Front) -> Self {
        self.lock().front = Some(front);
        self
    }

    /// The front end, when one is attached.
    pub fn front(&self) -> Option<Front> {
        self.lock().front.clone()
    }

    /// Run `f` with the real terminal: the front end, if any, steps aside meanwhile (an
    /// editor, a pager or a login prompt needs the whole screen and the keyboard).
    pub fn with_terminal<T>(&self, f: impl FnOnce() -> T) -> T {
        match self.front() {
            Some(front) => front.with_terminal(f),
            None => f(),
        }
    }

    /// Ring the terminal bell (a long turn is over), on a live terminal only.
    pub fn bell(&self) {
        let mut g = self.lock();
        if let Some(front) = &g.front {
            front.send(FrontMsg::Bell);
            return;
        }
        if g.cfg.live {
            let _ = g.err.write_all(b"\x07");
            let _ = g.err.flush();
        }
    }

    /// Whether an approval question is waiting for its key.
    pub fn is_asking(&self) -> bool {
        self.lock().asking.is_some()
    }

    /// Whether keys are read during a turn.
    pub fn type_ahead(&self) -> bool {
        self.type_ahead
    }

    /// The message being typed changed: show it on the status line.
    pub fn set_draft(&self, text: &str) {
        let mut g = self.lock();
        g.draft = text.to_string();
        g.refresh();
    }

    /// A message typed during the turn, to run after it; `first` puts it before the others
    /// (a message sent with Esc). Said on its own line so it is not lost from view.
    pub fn queue_message(&self, message: String, first: bool) {
        let mut g = self.lock();
        let style = g.cfg.err_style;
        let shown = message.lines().next().unwrap_or_default().to_string();
        let more = if message.lines().count() > 1 { " …" } else { "" };
        let when = if first { "next, after the stop" } else { "after this turn" };
        let line = format!("  {} {}{more} {}", style.cyan("↳"), shown, style.dim(&format!("(queued: {when})")));
        if first {
            g.queued.push_front(message);
        } else {
            if let Some(tx) = g.steer.as_ref().filter(|_| steerable_text(&message)) {
                let _ = tx.send(message.clone());
            }
            g.queued.push_back(message);
        }
        g.emit_err(&line);
    }

    /// Offer a message typed during the turn to the model as part of it. `false`: the turn
    /// cannot take it (another provider, or it is over); the message keeps its place in the
    /// person's queue either way, and the turn reports it ([`Event::Steered`]) once taken.
    pub fn steer(&self, text: &str) -> bool {
        steerable_text(text) && self.lock().steer.as_ref().is_some_and(|tx| tx.send(text.to_string()).is_ok())
    }

    /// The person took `text` back out of the queue to edit it. If it was already offered to the
    /// turn ([`Console::steer`]) but not taken yet, the turn must not send it as well: it comes
    /// back through the queue, edited.
    pub fn withdraw_steer(&self, text: &str) {
        self.lock().withdrawn.push(text.to_string());
    }

    /// The next queued message, oldest first (one sent with Esc before the rest).
    pub fn take_queued(&self) -> Option<String> {
        self.lock().queued.pop_front()
    }

    /// Drop every queued message; how many there were.
    pub fn clear_queue(&self) -> usize {
        let mut g = self.lock();
        let n = g.queued.len();
        g.queued.clear();
        n
    }

    /// How many messages wait in the queue.
    pub fn queued_len(&self) -> usize {
        self.lock().queued.len()
    }

    /// Put back an unfinished message when the reader stops, for the next prompt.
    pub fn keep_draft(&self, text: &str) {
        self.lock().draft = text.to_string();
    }

    /// The unfinished message, once (the prompt starts with it, or the next reader does).
    pub fn take_draft(&self) -> String {
        std::mem::take(&mut self.lock().draft)
    }

    /// Let the person answer tool approvals by pressing a key. Without this the console
    /// has no one to ask.
    pub fn with_key_reader(mut self, keys: KeyReader) -> Self {
        self.keys = Some(keys);
        self
    }

    /// Whether a tool approval can be asked here (a key reader is attached). Without one every
    /// question would be denied, so the run should not ask the CLI to send any.
    pub fn can_ask(&self) -> bool {
        self.keys.is_some() || self.lock().front.is_some()
    }

    /// Whether the pager of an approval question is open. A Ctrl-C then belongs to the pager
    /// (it quits a search there), not to the turn.
    pub fn is_paging(&self) -> bool {
        *self.paging.borrow()
    }

    /// Resolves once no pager is open (at once when none is). The turn waits for it before it
    /// gives the terminal back to the prompt.
    pub async fn pager_closed(&self) {
        let mut rx = self.paging.subscribe();
        // The sender lives as long as `self`, so the wait cannot fail.
        let _ = rx.wait_for(|open| !*open).await;
    }

    /// The pager is about to own the screen: hold everything printed from now on.
    pub(crate) fn begin_paging(&self) {
        self.lock().held.get_or_insert_with(Vec::new);
        self.paging.send_replace(true);
    }

    /// The pager closed: print what was held, in order, and let the status line back.
    pub(crate) fn end_paging(&self) {
        let mut g = self.lock();
        for line in g.held.take().unwrap_or_default() {
            match line {
                HeldLine::Out(text) => g.emit_out(&text),
                HeldLine::Err(text) => g.emit_err(&text),
            }
        }
        drop(g);
        self.paging.send_replace(false);
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

    pub(super) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn out_style(&self) -> Style {
        self.lock().cfg.out_style
    }

    /// Whether the model's whole reasoning is printed (otherwise only its latest line shows
    /// on the working row).
    pub fn show_thinking(&self) -> bool {
        self.lock().show_thinking
    }

    /// Print the model's whole reasoning from now on, or stop. `/thinking` sets it and keeps
    /// the choice for the next session.
    pub fn set_show_thinking(&self, on: bool) {
        self.lock().show_thinking = on;
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
        if g.live.is_some() || g.front.is_some() {
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

    /// The route of the latest turn (its model and the router's reason), for `/why`.
    pub fn last_route(&self) -> Option<RouteFacts> {
        self.lock().last_route.clone()
    }

    /// Clear the screen and its scrollback, on a terminal only.
    pub fn clear_screen(&self) {
        let mut g = self.lock();
        if let Some(front) = &g.front {
            front.send(FrontMsg::ClearScreen);
            return;
        }
        if g.cfg.out_style.enabled {
            let _ = g.out.write_all(CLEAR_SCREEN.as_bytes());
            let _ = g.out.flush();
        }
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
        let lines_first = if g.front.is_some() { format!("\n{}", uifmt::turn_closing(summary, style)) } else { format!("  {}", uifmt::turn_summary(summary, style)) };
        let mut lines = vec![lines_first];
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
