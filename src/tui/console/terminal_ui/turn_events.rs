use super::*;

impl Inner {
    /// An event of the turn, laid out for the interactive screen (see [`crate::tui::render`]'s roles).
    pub(super) fn front_event(&mut self, e: &Event) {
        let (style, width) = (self.cfg.err_style, self.width());
        match e {
            Event::Tool(summary) => {
                self.flush_text(true);
                let activity = uifmt::relativize_paths(&uifmt::strip_ansi(summary), &self.cfg.project);
                self.set_activity(&activity);
                let name = uifmt::tool_name(summary).to_string();
                self.tools_waiting.push_back(name.clone());
                let row = uifmt::tool_row(summary, &self.cfg.project, style, width);
                self.fresh_turn = false;
                if !group_tool_runs() || stands_alone(&name) {
                    self.flush_run();
                    self.emit_err(&row);
                } else {
                    // The working row says what runs now; the run prints as one row when it ends.
                    self.group.call(&name);
                    self.run.push(HeldCall { tool: name, summary: summary.clone(), rows: Vec::new(), resolved: false, failed: false });
                }
            }
            Event::Thinking(t) => {
                self.flush_text(true);
                if self.show_thinking {
                    self.set_activity("Thinking");
                    let block = self.step(uifmt::thinking_block(t, style, width));
                    self.fresh_turn = false;
                    self.emit_err(&block);
                } else {
                    // The reasoning is noise once it is over, and the scrollback cannot be
                    // taken back: it lives on the working row (its latest line) and is gone
                    // with the turn.
                    let latest = uifmt::strip_ansi(t).lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_string();
                    self.set_activity(&if latest.is_empty() { "Thinking".to_string() } else { format!("Thinking · {latest}") });
                    // A one-line trace stays behind, so the scrollback shows that it thought.
                    if !latest.is_empty() {
                        let row = uifmt::thought_row(&latest, style, width);
                        self.fresh_turn = false;
                        self.emit_err(&row);
                    }
                }
            }
            Event::Edit { path, removed, added } if group_tool_runs() => {
                // Counted into the run's row (`Edited a.rs +3 −1`); `/diff` has the lines.
                self.discard_preview();
                self.group.edit(path, removed.len(), added.len());
            }
            Event::Edit { path, removed, added } => {
                self.discard_preview();
                self.flush_run();
                let start = edit_start_line(&self.cfg.project, path, removed, added);
                let preview = uifmt::edit_preview_at(removed, added, start, style, width);
                self.emit_err(&preview);
            }
            Event::ToolResult { ok, text } => {
                self.discard_preview();
                let tool = self.tools_waiting.pop_front().unwrap_or_default();
                let rows: Vec<String> = [uifmt::tool_result_row(&tool, *ok, text, style, width), uifmt::tool_output_fold(&tool, *ok, text, style, width)].into_iter().flatten().collect();
                // A held call takes its own result; any other prints at once.
                match self.run.iter_mut().find(|h| !h.resolved && h.tool == tool) {
                    Some(held) => {
                        held.resolved = true;
                        held.failed = uifmt::tool_failed(&tool, *ok, text);
                        if held.failed {
                            held.rows = uifmt::tool_failure_rows(&held.summary, *ok, text, &self.cfg.project, style, width);
                        }
                    }
                    None => {
                        for row in rows {
                            self.emit_err(&row);
                        }
                    }
                }
            }
            // That a call went through unasked is what the mode in the footer already
            // says; under every call it was only noise.
            Event::Hint(t) if !is_allow_note(t) => {
                let text = hint_line(t, style, width);
                self.flush_run();
                self.emit_err(&text);
            }
            Event::Steered(text) => {
                self.flush_run();
                self.flush_text(true);
                if let Some(front) = &self.front {
                    front.send(FrontMsg::Steered(text.clone()));
                }
                self.fresh_turn = false;
                let line = steered_line(text, style);
                self.emit_err(&line);
            }
            Event::Text(t) if self.cfg.stream_answers => {
                // Two texts in a row: the first was part of the answer.
                self.flush_text(false);
                self.pending_text = Some(t.clone());
            }
            // The answer as it is written. Preview only: the whole text still arrives as
            // `Event::Text` and is what gets committed.
            Event::TextDelta(t) if self.cfg.stream_answers => {
                self.live_answer.push_str(t);
                if let Some(front) = &self.front {
                    front.send(FrontMsg::AnswerDelta(t.clone()));
                }
            }
            _ => {}
        }
    }

    /// Drop the answer as it was being written. Called before anything that turns the text
    /// into something else: a tool (it was narration), a failure, a cancel, or the commit.
    pub(super) fn discard_preview(&mut self) {
        if self.live_answer.is_empty() {
            return;
        }
        self.live_answer.clear();
        if let Some(front) = &self.front {
            front.send(FrontMsg::AnswerReset);
        }
    }

    pub(super) fn set_activity(&mut self, text: &str) {
        if let Some(front) = &self.front {
            front.send(FrontMsg::Activity(text.to_string()));
        }
        if let Some(l) = self.live.as_mut() {
            l.activity = Some(text.to_string());
        }
    }

    pub(super) fn failover_line(&self, p: &Pending, to: Option<(&str, &str)>) -> String {
        format!("  {}", uifmt::failover_note(&p.provider, &p.model, &p.kind, to, p.discarded, self.cfg.err_style))
    }

    pub(super) fn route(&mut self, f: &RouteFacts) {
        self.last_route = Some(f.clone());
        if let Some(p) = self.pending.take() {
            let line = self.failover_line(&p, Some((&f.provider, &f.model)));
            self.emit_err(&line);
        }
        let label = if f.model.is_empty() { f.provider.clone() } else { format!("{} · {}", f.provider, f.model) };
        self.flush_run();
        if let Some(front) = self.front.clone() {
            // The router at work leads the turn: where it went and why, in one row.
            front.send(FrontMsg::Label(label));
            let row = uifmt::route_row(f, self.cfg.err_style, self.width());
            let row = if self.fresh_turn { row } else { format!("\n{row}") };
            self.fresh_turn = true;
            self.emit_err(&row);
            return;
        }
        if let Some(l) = self.live.as_mut() {
            l.label = label;
            l.activity = None;
        }
        let line = format!("  {}", uifmt::route_line(f, self.cfg.err_style));
        self.emit_err(&line);
    }
}
