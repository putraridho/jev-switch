//! Keys while an approval card or a confirm question is up.

use super::*;

impl App {
    pub(super) fn confirm_key(&mut self, k: KeyEvent) -> Vec<Action> {
        let Some(c) = self.confirm.as_mut() else { return Vec::new() };
        let answer = match k.code {
            KeyCode::Char('y' | 'Y') => Some(true),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(false),
            KeyCode::Char('c') if ctrl(&k) => Some(false),
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                c.selected = 1 - c.selected;
                None
            }
            KeyCode::Enter => Some(c.selected == 0),
            _ => None,
        };
        if let Some(yes) = answer {
            c.answer(yes);
            self.confirm = None;
        }
        Vec::new()
    }

    pub(super) fn approval_key(&mut self, k: KeyEvent, now: Instant) -> Vec<Action> {
        let Some(a) = self.approval.as_mut() else { return Vec::new() };
        let stop = matches!(k.code, KeyCode::Char('c')) && ctrl(&k);
        if !stop && now.saturating_duration_since(a.shown_at) < CARD_GRACE {
            return Vec::new();
        }
        let choices = a.choices();
        let pick = |key: AnswerKey| choices.iter().any(|(c, _)| *c == key).then_some(key);
        let answer = match k.code {
            KeyCode::Char('c') if ctrl(&k) => Some(AnswerKey::Interrupt),
            KeyCode::Char('y' | 'Y') => Some(AnswerKey::Yes),
            KeyCode::Char('a' | 'A') => pick(AnswerKey::Always),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(AnswerKey::No),
            KeyCode::Char(c @ '1'..='9') => c.to_digit(10).and_then(|d| choices.get(d as usize - 1)).map(|(key, _)| *key),
            KeyCode::Up => {
                a.selected = (a.selected + choices.len() - 1) % choices.len();
                None
            }
            KeyCode::Down | KeyCode::Tab => {
                a.selected = (a.selected + 1) % choices.len();
                None
            }
            KeyCode::PageUp => {
                a.scroll = a.scroll.saturating_sub(5);
                None
            }
            KeyCode::PageDown => {
                a.scroll = (a.scroll + 5).min(a.lines.len().saturating_sub(1));
                None
            }
            KeyCode::Enter => choices.get(a.selected).map(|(key, _)| *key),
            _ => None,
        };
        if let Some(key) = answer {
            a.answer(key);
            self.approval = None;
        }
        Vec::new()
    }
}
