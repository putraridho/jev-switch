//! The suggestion and menu side of the box: ghost text, the command menu and what its keys do.

use super::*;

impl App {
    /// The dim text after the cursor that Right would put in: the suggested next prompt in an
    /// empty box, else the rest of a sent message (or an `@file`) that starts with what is
    /// typed. Only at the end of a one-line text, and never over a menu, card or history.
    pub fn ghost(&self) -> Option<String> {
        if self.menu.is_some() || self.approval.is_some() || self.confirm.is_some() || self.history.browsing() {
            return None;
        }
        let text = self.editor.text();
        if text.is_empty() {
            return self.suggestion.clone().filter(|_| self.working.is_none());
        }
        if self.editor.cursor() != text.len() || text.contains('\n') {
            return None;
        }
        crate::input::ghost::ghost_suffix(text, self.history.newest_first(), &self.tracked)
    }

    /// Right at the end of the text: take the ghost text. False when there is none.
    pub(super) fn accept_ghost(&mut self) -> bool {
        let Some(rest) = self.ghost() else { return false };
        self.editor.insert_str(&rest);
        self.edited();
        true
    }

    pub(super) fn refresh_menu(&mut self) {
        if self.menu_closed_for.as_deref() == Some(self.editor.text()) {
            self.menu = None;
            return;
        }
        self.menu_closed_for = None;
        let at_end = self.editor.cursor() == self.editor.text().len();
        self.menu = if at_end { menu::build(self.editor.before_cursor(), &self.ctx, &self.project, &self.tracked) } else { None };
    }

    /// The text changed by a key: browsing history stops and the menu follows the text.
    pub(super) fn edited(&mut self) {
        self.history.reset();
        self.refresh_menu();
    }
    /// Keys the open menu takes; `None` passes the key on to the editor.
    pub(super) fn menu_key(&mut self, k: &KeyEvent) -> Option<Vec<Action>> {
        let m = self.menu.as_mut()?;
        match k.code {
            KeyCode::Up => m.up(),
            KeyCode::Down => m.down(),
            KeyCode::Char('p') if ctrl(k) => m.up(),
            KeyCode::Char('n') if ctrl(k) => m.down(),
            KeyCode::Esc => {
                self.menu = None;
                self.menu_closed_for = Some(self.editor.text().to_string());
            }
            KeyCode::Tab => {
                self.accept(false);
            }
            KeyCode::Enter if k.modifiers.is_empty() => {
                if self.accept(true) {
                    return Some(self.submit());
                }
            }
            _ => return None,
        }
        Some(Vec::new())
    }

    /// Put the selected item in the text. True when Enter should run the line now.
    fn accept(&mut self, enter: bool) -> bool {
        let Some(m) = self.menu.take() else { return false };
        let Some(item) = m.current().cloned() else { return false };
        let typed = self.editor.before_cursor()[m.from..].to_string();
        let already = typed == item.replacement;
        // Enter runs what was typed when it is complete already (`/permissions`, `/resume`),
        // and never swaps in a different command found by scattered letters (`/provders` is
        // sent as typed, to be told what was meant) unless the person picked it.
        if enter && (already || (!m.navigated && !item.replacement.to_lowercase().starts_with(&typed.to_lowercase()))) {
            return true;
        }
        let suffix = match item.then {
            Then::Arguments => " ",
            Then::Run | Then::Stay => "",
        };
        self.editor.replace_before_cursor(m.from, &format!("{}{suffix}", item.replacement));
        self.edited();
        match item.then {
            Then::Run => enter,
            // A file path chosen with Enter is put in; Enter again on the same text sends.
            Then::Stay => enter && already,
            Then::Arguments => false,
        }
    }

}
