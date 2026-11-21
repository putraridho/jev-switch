//! One key press: routing to the open dialog or menu, and the box's own editing keys.

use super::*;

impl App {
    /// One key press.
    pub fn on_key(&mut self, k: KeyEvent, now: Instant) -> Vec<Action> {
        if k.kind == KeyEventKind::Release {
            return Vec::new();
        }
        if self.approval.as_ref().is_some_and(Approval::withdrawn) {
            self.approval = None;
        }
        if self.confirm.is_some() {
            return self.confirm_key(k);
        }
        if self.approval.is_some() {
            return self.approval_key(k, now);
        }
        if self.menu.is_some() {
            if let Some(actions) = self.menu_key(&k) {
                return actions;
            }
        }
        self.edit_key(k, now)
    }

    /// Enter: send the text, or queue it while a turn runs.
    pub(super) fn submit(&mut self) -> Vec<Action> {
        let text = self.editor.take();
        self.menu = None;
        self.menu_closed_for = None;
        if text.trim().is_empty() {
            return Vec::new();
        }
        // Anything sent answers the last answer: its offer no longer stands.
        self.suggestion = None;
        self.history.remember(&text);
        if self.working.is_some() {
            self.queue.push_back(text.clone());
            // The turn may take it in right away; it stays queued until the model has it.
            return vec![Action::Steer(text)];
        }
        vec![Action::Submit(text)]
    }

    fn newline(&mut self) {
        self.editor.insert_char('\n');
        self.edited();
    }

    fn edit_key(&mut self, k: KeyEvent, now: Instant) -> Vec<Action> {
        let busy = self.working.is_some();
        if let Some(action) = self.keymap.action(&k) {
            return match action {
                UserAction::Clear => vec![Action::ClearScreen],
                UserAction::CycleMode => {
                    self.mode = self.mode.next();
                    vec![Action::Mode(self.mode)]
                }
                UserAction::Quit => vec![Action::Quit],
            };
        }
        if let Some(vim) = self.vim.as_mut() {
            match vim.on_key(&mut self.editor, &k, busy) {
                Outcome::Pass => {}
                Outcome::Moved => return Vec::new(),
                Outcome::Edited => {
                    self.edited();
                    return Vec::new();
                }
            }
        }
        match k.code {
            // `?` on an empty box opens the key cheat sheet.
            KeyCode::Char('?') if self.editor.is_empty() && !busy && !ctrl(&k) => {
                self.editor.set("/keys");
                return self.submit();
            }
            KeyCode::Enter if k.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => self.newline(),
            KeyCode::Char('j') if ctrl(&k) => self.newline(),
            KeyCode::Enter => {
                // A line ending in `\` continues on the next one, as in a shell.
                if self.editor.before_cursor().ends_with('\\') && self.editor.cursor() == self.editor.text().len() {
                    self.editor.backspace();
                    self.newline();
                } else {
                    return self.submit();
                }
            }
            KeyCode::Char('c') if ctrl(&k) => return self.ctrl_c(now),
            KeyCode::Esc if busy => {
                // Stop now; what was typed goes first, right after the stop.
                let text = self.editor.take();
                if !text.trim().is_empty() {
                    self.history.remember(&text);
                    self.queue.push_front(text);
                }
                self.edited();
                return vec![Action::Stop];
            }
            KeyCode::Esc => {}
            KeyCode::Char('d') if ctrl(&k) => {
                if self.editor.is_empty() && !busy {
                    return vec![Action::Quit];
                }
                self.editor.delete();
                self.edited();
            }
            KeyCode::BackTab => {
                self.mode = self.mode.next();
                return vec![Action::Mode(self.mode)];
            }
            KeyCode::Char('l') if ctrl(&k) => return vec![Action::ClearScreen],
            KeyCode::Char('r') if ctrl(&k) => {
                let needle = self.editor.text().to_string();
                if let Some(found) = self.history.search(&needle, &needle) {
                    self.editor.set(&found);
                    self.menu = None;
                }
            }
            KeyCode::Up if !self.editor.up() => {
                // A message sent during the turn that is still queued comes back before
                // history does: ↑ takes the newest one out of the queue. Only into an
                // empty box, so a draft is never thrown away for it.
                if self.editor.is_empty() {
                    if let Some(queued) = self.queue.pop_back() {
                        self.editor.set(&queued);
                        self.menu = None;
                        return Vec::new();
                    }
                }
                if let Some(older) = self.history.older(self.editor.text()) {
                    self.editor.set(&older);
                    self.menu = None;
                }
            }
            KeyCode::Down if !self.editor.down() => {
                if let Some(newer) = self.history.newer() {
                    self.editor.set(&newer);
                    self.menu = None;
                }
            }
            KeyCode::Left if ctrl(&k) || alt(&k) => self.editor.word_left(),
            KeyCode::Right if ctrl(&k) || alt(&k) => self.editor.word_right(),
            KeyCode::Char('b') if alt(&k) => self.editor.word_left(),
            KeyCode::Char('f') if alt(&k) => self.editor.word_right(),
            KeyCode::Left => self.editor.left(),
            KeyCode::Right if !self.accept_ghost() => self.editor.right(),
            KeyCode::Char('b') if ctrl(&k) => self.editor.left(),
            KeyCode::Char('f') if ctrl(&k) => self.editor.right(),
            KeyCode::PageUp => self.scroll_chat(self.pane_rows.saturating_sub(2).max(1) as isize),
            KeyCode::PageDown => self.scroll_chat(-(self.pane_rows.saturating_sub(2).max(1) as isize)),
            KeyCode::End if ctrl(&k) => self.transcript.to_end(),
            KeyCode::Home if ctrl(&k) => self.transcript.to_start(self.pane_rows),
            KeyCode::Home => self.editor.home(),
            KeyCode::End => self.editor.end(),
            KeyCode::Char('a') if ctrl(&k) => self.editor.home(),
            KeyCode::Char('e') if ctrl(&k) => self.editor.end(),
            KeyCode::Backspace if alt(&k) || ctrl(&k) => {
                self.editor.delete_word_back();
                self.edited();
            }
            KeyCode::Char('w') if ctrl(&k) => {
                self.editor.delete_word_back();
                self.edited();
            }
            KeyCode::Char('k') if ctrl(&k) => {
                self.editor.kill_to_end();
                self.edited();
            }
            KeyCode::Char('u') if ctrl(&k) => {
                self.editor.kill_to_start();
                self.edited();
            }
            KeyCode::Char('h') if ctrl(&k) => {
                self.editor.backspace();
                self.edited();
            }
            KeyCode::Backspace => {
                self.editor.backspace();
                self.edited();
            }
            KeyCode::Delete => {
                self.editor.delete();
                self.edited();
            }
            KeyCode::Tab => self.refresh_menu(),
            KeyCode::Char(c) if !ctrl(&k) => {
                self.editor.insert_char(c);
                self.edited();
            }
            _ => {}
        }
        // Any key but Ctrl-C ends the "press again to exit" window.
        self.last_ctrl_c = None;
        Vec::new()
    }

    /// Ctrl-C: stop a running turn (and drop the queue); at the prompt, clear the text, and on
    /// an empty box twice within the window, exit.
    fn ctrl_c(&mut self, now: Instant) -> Vec<Action> {
        if self.working.is_some() {
            let dropped = self.queue.len();
            self.queue.clear();
            let mut out = vec![Action::Stop];
            if dropped > 0 {
                let noun = if dropped == 1 { "message" } else { "messages" };
                out.push(Action::Print(format!("  dropped {dropped} queued {noun} (Ctrl-C); Esc stops and keeps them")));
            }
            return out;
        }
        if !self.editor.is_empty() {
            self.editor.take();
            self.edited();
            self.last_ctrl_c = None;
            return Vec::new();
        }
        if self.last_ctrl_c.is_some_and(|t| now.duration_since(t) < INTERRUPT_WINDOW) {
            return vec![Action::Quit];
        }
        self.last_ctrl_c = Some(now);
        self.notice = Some(("Press Ctrl-C again to exit".into(), now));
        Vec::new()
    }
}
