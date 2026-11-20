use super::*;

impl Inner {
    /// Columns right now: what the terminal reports, else the configured width.
    pub(super) fn width(&self) -> usize {
        let cols = self.screen.as_ref().and_then(|s| s.columns()).unwrap_or(self.cfg.width).max(1);
        // The front end's chat pane may be narrower than the terminal.
        terminal::width_cap().map_or(cols, |cap| cols.min(cap)).max(1)
    }

    /// Remove the live area (status line and field) from the screen; it is redrawn after the
    /// next print. The cursor ends on the first row of it, where the next line goes.
    pub(super) fn erase(&mut self) {
        if let Some(l) = self.live.as_mut().filter(|l| l.rows > 0) {
            let rows = std::mem::take(&mut l.rows);
            let mut frame = String::from(uifmt::clear_line());
            for _ in 1..rows {
                frame.push_str(uifmt::up_row());
                frame.push_str(uifmt::clear_line());
            }
            let _ = self.err.write_all(frame.as_bytes());
        }
    }

    pub(super) fn draw(&mut self, elapsed: Duration) {
        if self.asking.is_some() || self.held.is_some() {
            return;
        }
        // Each row is redrawn in place (`\r`, erase the row), which only works while it is ONE
        // screen row: so it is cut by display width to one column short of the live width,
        // since a full row wraps on terminals that wrap at the last column.
        let budget = self.width().saturating_sub(1);
        let style = self.cfg.err_style;
        let field = self.type_ahead.then(|| uifmt::input_row(&self.draft, self.queued.len(), budget, style));
        let Some(l) = self.live.as_mut() else { return };
        let line = uifmt::status_line(&l.label, elapsed, l.activity.as_deref(), budget, style);
        let mut frame = String::new();
        if !self.cursor_hidden {
            self.cursor_hidden = true;
            terminal::note_cursor_hidden(true);
            frame.push_str(uifmt::hide_cursor());
        }
        // The cursor is on the last row of what was drawn: go up to the first, then redraw.
        if l.rows > 1 {
            frame.push_str(uifmt::up_row());
        }
        frame.push_str(uifmt::clear_line());
        frame.push_str(&line);
        l.rows = 1;
        if let Some(field) = field {
            frame.push_str("\r\n\x1b[2K");
            frame.push_str(&field);
            l.rows = 2;
        }
        let _ = self.err.write_all(frame.as_bytes());
        let _ = self.err.flush();
    }

    pub(super) fn refresh(&mut self) {
        if let Some(elapsed) = self.live.as_ref().map(|l| l.started.elapsed()) {
            self.draw(elapsed);
        }
    }

    /// Something other than the question was printed: its rows are no longer the last on the
    /// screen, so they can no longer be erased by counting up.
    pub(super) fn disturb_question(&mut self) {
        if let Some(rows) = self.asking.as_mut() {
            *rows = 0;
        }
    }

    pub(super) fn emit_err(&mut self, text: &str) {
        if let Some(front) = &self.front {
            front.send(FrontMsg::Print(text.strip_suffix('\n').unwrap_or(text).to_string()));
            return;
        }
        if let Some(held) = self.held.as_mut() {
            held.push(HeldLine::Err(text.to_string()));
            return;
        }
        self.disturb_question();
        self.erase();
        let _ = self.err.write_all(text.as_bytes());
        if !text.ends_with('\n') {
            let _ = self.err.write_all(b"\n");
        }
        let _ = self.err.flush();
        self.refresh();
    }

    pub(super) fn emit_out(&mut self, text: &str) {
        if let Some(front) = &self.front {
            front.send(FrontMsg::Print(text.strip_suffix('\n').unwrap_or(text).to_string()));
            return;
        }
        if let Some(held) = self.held.as_mut() {
            held.push(HeldLine::Out(text.to_string()));
            return;
        }
        self.disturb_question();
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

    pub(super) fn begin(&mut self) {
        self.fresh_turn = true;
        self.discard_preview();
        self.run.clear();
        self.group = uifmt::ToolGroup::default();
        if let Some(front) = &self.front {
            front.send(FrontMsg::TurnStarted);
            return;
        }
        if !self.cfg.live {
            return;
        }
        self.live = Some(Live { label: "routing".into(), activity: None, started: Instant::now(), rows: 0 });
        if let Some(screen) = &self.screen {
            // Remember what the terminal had, so `end` puts back exactly that: a person who
            // runs with echo already off must not find it switched on.
            self.echo_before = screen.echo();
            if self.echo_before == Some(true) {
                screen.set_echo(false);
            }
        }
        self.refresh();
    }

    pub(super) fn end(&mut self) {
        // A withdrawn question's key reader may still be restoring the terminal mode.
        settle_key_readers_from_async();
        self.erase();
        self.live = None;
        self.asking = None;
        if self.cursor_hidden {
            self.cursor_hidden = false;
            terminal::note_cursor_hidden(false);
            let _ = self.err.write_all(uifmt::show_cursor().as_bytes());
        }
        if let (Some(was_on), Some(screen)) = (self.echo_before.take(), &self.screen) {
            if was_on {
                screen.set_echo(true);
            }
        }
        let _ = self.err.flush();
        if let Some(p) = self.pending.take() {
            let line = self.failover_line(&p, None);
            self.emit_err(&line);
        }
        if self.front.is_some() {
            self.flush_run();
            self.flush_text(false);
            self.tools_waiting.clear();
        }
        if let Some(front) = &self.front {
            front.send(FrontMsg::TurnEnded);
        }
    }

    /// The run of tool calls is over: print it as one row that reads as a sentence
    /// (`◇ Edited parser.rs, explored 3 files, ran 2 commands +12 −4`), then the calls among
    /// it that failed, in full, so what went wrong is never folded away.
    pub(super) fn flush_run(&mut self) {
        let mut group = std::mem::take(&mut self.group);
        let run = std::mem::take(&mut self.run);
        if group.is_empty() && run.is_empty() {
            return;
        }
        let (style, width) = (self.cfg.err_style, self.width());
        for _ in run.iter().filter(|h| h.failed) {
            group.failed();
        }
        let mut rows = vec![uifmt::tool_group_row(&group, style, width)];
        rows.extend(run.into_iter().filter(|h| h.failed).flat_map(|h| h.rows));
        rows.retain(|r| !r.is_empty());
        if !rows.is_empty() {
            self.emit_err(&rows.join("\n"));
        }
    }

    /// Print the held assistant text: as narration (a tool call followed it) or as the answer.
    pub(super) fn flush_text(&mut self, narration: bool) {
        // Whatever was being written is finished (or was not the answer): the preview goes
        // before the text is laid out below.
        self.discard_preview();
        let Some(text) = self.pending_text.take() else { return };
        self.flush_run();
        let (style, width) = (self.cfg.out_style, self.width());
        let shown = if narration && !show_narration() {
            // Between tool calls only the calls show; what the model said lives on the
            // working row for as long as it is the latest thing.
            let latest = uifmt::strip_ansi(&text).lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_string();
            if !latest.is_empty() {
                self.set_activity(&latest);
            }
            return;
        } else if narration {
            self.step(uifmt::narration(&text, style, width))
        } else if let Some(route) = self.last_route.as_ref().filter(|_| attribution_gutter_enabled()) {
            // The answer says who wrote it, so a mid-session switch shows at a glance.
            let laid_out = uifmt::answer_text(&text, style, width.saturating_sub(2));
            uifmt::ProviderTheme::from_env(style).attributed_answer(&[(route.provider.as_str(), route.model.as_str())], &laid_out)
        } else {
            // On the interactive screen the answer shares the margin of the rows around it.
            if self.front.is_some() {
                uifmt::answer_panel(&uifmt::answer_text(&text, style, width.saturating_sub(4)), style)
            } else {
                uifmt::answer_block(&uifmt::answer_text(&text, style, width), 0)
            }
        };
        self.fresh_turn = false;
        self.emit_out(&shown);
    }

    /// A new step of the turn (the model thinking or saying what it does next) starts after a
    /// blank line, so a turn reads as steps; the first one follows the message directly.
    pub(super) fn step(&self, block: String) -> String {
        if self.fresh_turn {
            block
        } else {
            format!("\n{block}")
        }
    }
}
