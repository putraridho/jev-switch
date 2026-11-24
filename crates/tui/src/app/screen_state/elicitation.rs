//! Editing a server's form without changing queued messages or permission rules.

use super::*;
use jev_backend::{ElicitationRequest, ElicitationAnswer, ElicitationKind, ElicitationField, FieldKind};

#[derive(Debug)]
pub struct Elicitation {
    pub request: ElicitationRequest,
    reply: Option<oneshot::Sender<ElicitationAnswer>>,
    pub values: Vec<String>,
    pub field: usize,
    pub editor: Editor,
    pub selected: usize,
    pub error: Option<String>,
    pub scroll: usize,
    shown_at: Instant,
}

impl Elicitation {
    pub(super) fn new(request: ElicitationRequest, reply: oneshot::Sender<ElicitationAnswer>, now: Instant) -> Self {
        let values = match &request.kind {
            ElicitationKind::Form(fields) => fields.iter().map(|f| f.default.clone()).collect(),
            _ => Vec::new(),
        };
        let mut editor = Editor::default();
        if let Some(first) = values.first() { editor.set(first); }
        Self { request, reply: Some(reply), values, field: 0, editor, selected: 1, error: None, scroll: 0, shown_at: now }
    }

    pub fn fields(&self) -> &[ElicitationField] {
        match &self.request.kind { ElicitationKind::Form(f) => f, _ => &[] }
    }

    pub fn editing(&self) -> bool { self.field < self.fields().len() }

    pub(super) fn withdrawn(&self) -> bool { self.reply.as_ref().is_none_or(oneshot::Sender::is_closed) }

    pub fn can_accept(&self) -> bool { !matches!(self.request.kind, ElicitationKind::Unsupported(_)) }

    pub fn choices(&self) -> Vec<&'static str> {
        if self.can_accept() { vec!["Send answer", "Decline", "Cancel"] } else { vec!["Decline", "Cancel"] }
    }

    fn answer(&mut self, answer: ElicitationAnswer) {
        if let Some(reply) = self.reply.take() { let _ = reply.send(answer); }
    }

    fn advance(&mut self) {
        if self.editing() {
            if let Err(error) = self.fields()[self.field].parse(self.editor.text()) {
                self.error = Some(error);
                return;
            }
            self.values[self.field] = self.editor.take();
            self.field += 1;
            if let Some(value) = self.values.get(self.field) { self.editor.set(value); }
            self.error = None;
        }
    }

    fn back(&mut self) {
        if self.field > 0 {
            if self.editing() { self.values[self.field] = self.editor.take(); }
            self.field -= 1;
            self.editor.set(&self.values[self.field]);
            self.error = None;
        }
    }

    fn accept(&mut self) -> Option<ElicitationAnswer> {
        let mut content = Vec::new();
        for (i, field) in self.fields().iter().enumerate() {
            match field.parse(&self.values[i]) {
                Ok(Some(value)) => content.push((field.name.clone(), value)),
                Ok(None) => {},
                Err(error) => {
                    self.field = i;
                    self.editor.set(&self.values[i]);
                    self.error = Some(error);
                    return None;
                }
            }
        }
        Some(ElicitationAnswer::Accept(content))
    }

    fn cycle(&mut self, forward: bool) {
        let choices = match &self.fields()[self.field].kind {
            FieldKind::Boolean => vec!["false".into(), "true".into()],
            FieldKind::Choice(v) => v.clone(),
            _ => return,
        };
        let at = choices.iter().position(|c| c == self.editor.text());
        let at = match at {
            None => 0,
            Some(i) if forward => (i + 1) % choices.len(),
            Some(i) => (i + choices.len() - 1) % choices.len(),
        };
        self.editor.set(&choices[at]);
        self.error = None;
    }
}

impl App {
    pub(super) fn elicitation_key(&mut self, key: KeyEvent, now: Instant) -> Vec<Action> {
        let Some(e) = self.elicitation.as_mut() else { return Vec::new() };
        let stop = key.code == KeyCode::Char('c') && ctrl(&key);
        if !stop && now.saturating_duration_since(e.shown_at) < CARD_GRACE { return Vec::new(); }
        let mut answer = None;
        match key.code {
            KeyCode::Char('c') if ctrl(&key) => answer = Some(ElicitationAnswer::Cancel),
            KeyCode::Esc => answer = Some(ElicitationAnswer::Cancel),
            KeyCode::PageUp => e.scroll = e.scroll.saturating_sub(5),
            KeyCode::PageDown => e.scroll = e.scroll.saturating_add(5),
            KeyCode::BackTab => e.back(),
            KeyCode::Enter | KeyCode::Tab if e.editing() => e.advance(),
            KeyCode::Enter => {
                answer = if e.can_accept() && e.selected == 0 { e.accept() }
                    else if (e.can_accept() && e.selected == 1) || (!e.can_accept() && e.selected == 0) { Some(ElicitationAnswer::Decline) }
                    else { Some(ElicitationAnswer::Cancel) };
            }
            KeyCode::Up | KeyCode::Down if e.editing() => e.cycle(key.code == KeyCode::Down),
            KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                let len = e.choices().len();
                e.selected = (e.selected + if key.code == KeyCode::Up { len - 1 } else { 1 }) % len;
            }
            KeyCode::Char(c) if e.editing() && !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) && !c.is_control() => { e.editor.insert_char(c); e.error = None; }
            KeyCode::Backspace if e.editing() => e.editor.backspace(),
            KeyCode::Delete if e.editing() => e.editor.delete(),
            KeyCode::Left if e.editing() => e.editor.left(),
            KeyCode::Right if e.editing() => e.editor.right(),
            KeyCode::Home if e.editing() => e.editor.home(),
            KeyCode::End if e.editing() => e.editor.end(),
            _ => {},
        }
        if let Some(answer) = answer {
            e.answer(answer);
            self.elicitation = None;
        }
        if stop { vec![Action::Stop] } else { Vec::new() }
    }
}
