use super::*;
use std::path::Path;

mod dialogs;
mod elicitation;
mod ghost;
mod keys;
mod menu;

pub(super) fn app() -> App {
    let mut a = App::new(Path::new("/nonexistent").into(), History::from_entries(&["older message"]), Mode::Edit, "auto".into(), Vec::new());
    a.ctx = CompleteCtx { ready_providers: vec!["claude".into()], ..CompleteCtx::default() };
    a
}

pub(super) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub(super) fn with(code: KeyCode, m: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, m)
}

pub(super) fn typed(a: &mut App, text: &str) -> Vec<Action> {
    let mut out = Vec::new();
    for c in text.chars() {
        out.extend(a.on_key(key(KeyCode::Char(c)), Instant::now()));
    }
    out
}

pub(super) fn submitted(actions: &[Action]) -> Option<&str> {
    actions.iter().find_map(|a| match a {
        Action::Submit(t) => Some(t.as_str()),
        _ => None,
    })
}
