use super::{Action, Key, McpPicker, Outcome, Screen};

impl McpPicker {
    pub(super) fn press_list(&mut self, key: Key) -> Outcome {
        let shown = self.visible();
        let last = shown.len().saturating_sub(1);
        match key {
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => self.selected = (self.selected + 1).min(last),
            Key::PageUp => self.selected = self.selected.saturating_sub(self.page),
            Key::PageDown => self.selected = (self.selected + self.page).min(last),
            Key::Enter | Key::Right => {
                if let Some(&server) = shown.get(self.selected) {
                    self.screen = Screen::Detail { server, action: 0 };
                }
            }
            // Esc first clears a search, then leaves.
            Key::Cancel if !self.filter.is_empty() => {
                self.filter.clear();
                self.selected = 0;
            }
            Key::Cancel | Key::Interrupt => return Outcome::Cancelled,
            Key::Left => {}
            Key::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            Key::Char(c) => {
                self.filter.push(c);
                self.selected = 0;
            }
        }
        self.keep_cursor_on_page();
        Outcome::Continue
    }

    pub(super) fn press_detail(&mut self, key: Key, server: usize, action: usize) -> Outcome {
        let actions = Self::actions(&self.servers[server]);
        let last = actions.len().saturating_sub(1);
        match key {
            Key::Up => {
                self.screen = Screen::Detail {
                    server,
                    action: action.saturating_sub(1),
                }
            }
            Key::Down => {
                self.screen = Screen::Detail {
                    server,
                    action: (action + 1).min(last),
                }
            }
            Key::Enter => match actions.get(action) {
                Some(Action::Login(scope)) => {
                    return Outcome::Login {
                        server: self.servers[server].name.clone(),
                        scope: *scope,
                    }
                }
                Some(Action::Back) | None => self.screen = Screen::List,
            },
            Key::Left | Key::Cancel | Key::Backspace => self.screen = Screen::List,
            Key::Interrupt => return Outcome::Cancelled,
            Key::Right | Key::PageUp | Key::PageDown | Key::Char(_) => {}
        }
        Outcome::Continue
    }

    fn keep_cursor_on_page(&mut self) {
        let count = self.visible().len();
        if count <= self.page {
            self.offset = 0;
        } else if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + self.page {
            self.offset = self.selected + 1 - self.page;
        }
    }
}
