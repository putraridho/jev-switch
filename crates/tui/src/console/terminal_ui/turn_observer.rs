use super::*;

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
        if g.front.is_some() {
            g.front_event(e);
            return;
        }
        match e {
            Event::Tool(summary) => {
                let (style, width) = (g.cfg.err_style, g.width());
                let line = uifmt::tool_line(summary, &g.cfg.project, style, width);
                let activity = uifmt::relativize_paths(&uifmt::strip_ansi(summary), &g.cfg.project);
                g.set_activity(&activity);
                g.emit_err(&line);
            }
            // Diagnostics, never part of the answer: stderr, in print mode too.
            Event::Hint(t) => {
                let (style, width) = (g.cfg.err_style, g.width());
                let text = hint_line(t, style, width);
                g.emit_err(&text);
            }
            // The whole reasoning, when asked for and nowhere else: it is not the answer.
            Event::Thinking(t) if g.show_thinking => {
                let (style, width) = (g.cfg.err_style, g.width());
                let block = g.step(uifmt::thinking_block(t, style, width));
                g.fresh_turn = false;
                g.emit_err(&block);
            }
            Event::Text(t) if g.cfg.stream_answers => {
                let text = uifmt::answer_text(t, g.cfg.out_style, g.width());
                g.emit_out(&format!("{text}\n"));
            }
            // The model has it: it no longer waits for the next turn.
            Event::Steered(text) => {
                if let Some(at) = g.queued.iter().position(|q| q == text) {
                    g.queued.remove(at);
                }
                let line = steered_line(text, g.cfg.err_style);
                g.emit_err(&line);
            }
            _ => {}
        }
    }

    fn steering(&mut self) -> Option<tokio::sync::mpsc::UnboundedReceiver<String>> {
        let mut g = self.lock();
        if !(self.type_ahead || g.front.is_some()) {
            return None;
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        g.steer = Some(tx);
        // A withdrawal only concerns messages offered to this attempt.
        g.withdrawn.clear();
        Some(rx)
    }

    fn steer_wanted(&mut self) -> Option<SteerFilter> {
        let console = self.clone();
        Some(Box::new(move |text| {
            let mut g = console.lock();
            match g.withdrawn.iter().position(|w| w == text) {
                Some(at) => {
                    g.withdrawn.remove(at);
                    false
                }
                None => true,
            }
        }))
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
        // A failed attempt's last words are not an answer.
        g.pending_text = None;
        g.discard_preview();
        g.flush_run();
        g.tools_waiting.clear();
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
        // `claude · sonnet-5-5`, as the turn's header names them.
        let detail = format!("{} · {}: {}", f.provider, f.model, f.message.trim());
        let block = uifmt::error_block(f.kind, &detail, &uifmt::next_steps_for(f.kind, f.provider.as_str()), style);
        // Indented like the rest of the turn's rows (`  ◆ claude`, `  ⏺ Read`), so it reads as
        // part of the turn and not as a message from outside it.
        g.emit_err(&indented(&block));
    }

    fn no_route(&mut self, info: &NoRouteInfo, _text: &str) {
        let mut g = self.lock();
        match info {
            NoRouteInfo::Nothing { task } => g.no_route_task = Some((*task).to_string()),
            NoRouteInfo::Refused { why } => {
                let steps = ["See your rules and the audit trail: `/privacy`".to_string(), "See the spend and the cap: `/budget`".to_string()];
                let block = uifmt::problem_block("Your own rules left no model for this turn", &[one_sentence(why)], &steps, g.cfg.err_style);
                g.emit_err(&block);
            }
            NoRouteInfo::Pin { what } => {
                let steps = ["Let the router choose again: `/model auto`".to_string(), "See what can run now: `/providers`".to_string()];
                let block = uifmt::problem_block("Your /model pin can't run right now", &[one_sentence(what)], &steps, g.cfg.err_style);
                g.emit_err(&block);
            }
        }
    }

    /// The `■ stopped` summary says it; no extra line.
    fn cancelled(&mut self, _text: &str) {
        // The half-written answer is not one: it goes with the turn.
        self.lock().discard_preview();
    }

    /// Stops the status line, prints the question and returns the wait for one key.
    fn ask_permission(&mut self, req: &PermissionRequest, offer: &Offer) -> Answer {
        self.ask_with_pager(req, offer, Arc::new(page_on_terminal))
    }

    /// The answer line, printed only now that the engine has saved (or failed to save) what
    /// "always" saves, so it says what is true.
    fn permission_settled(&mut self, _req: &PermissionRequest, settled: &Settled) {
        let mut g = self.lock();
        let text = indented(&uifmt::permission_answered(settled, g.cfg.err_style));
        g.emit_err(&text);
    }

    /// Erases the question's rows (when it is the last thing on a terminal that takes escape
    /// sequences), says in one dim line that the tool call was withdrawn, and lets the
    /// status line resume.
    fn permission_withdrawn(&mut self, _req: &PermissionRequest) {
        let mut g = self.lock();
        let Some(rows) = g.asking.take() else { return };
        if let Some(front) = g.front.clone() {
            front.send(FrontMsg::ApprovalGone);
            let line = format!("  {}", g.cfg.err_style.dim(WITHDRAWN));
            g.emit_err(&line);
            return;
        }
        if rows > 0 && g.cfg.err_style.enabled {
            let _ = g.err.write_all(erase_rows_above(rows).as_bytes());
        }
        let line = format!("  {}", g.cfg.err_style.dim(WITHDRAWN));
        g.emit_err(&line);
    }
}
