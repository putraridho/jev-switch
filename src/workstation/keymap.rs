//! User keybindings: a small file of `key = action` lines that rebinds keys for a few
//! actions, e.g. `ctrl-o = clear`. Lines starting with `#` and blank lines are ignored; a
//! line that cannot be read is reported and skipped, never fatal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What a user key can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserAction {
    /// Clear the screen (Ctrl-L by default).
    Clear,
    /// Cycle the mode (Shift-Tab by default).
    CycleMode,
    /// Leave jev-code.
    Quit,
}

impl UserAction {
    fn parse(word: &str) -> Option<Self> {
        match word {
            "clear" => Some(Self::Clear),
            "cycle-mode" | "mode" => Some(Self::CycleMode),
            "quit" => Some(Self::Quit),
            _ => None,
        }
    }
}

/// A key and its modifiers, as written in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Chord {
    code: KeyCode,
    modifiers: KeyModifiers,
}

fn parse_chord(text: &str) -> Option<Chord> {
    let lower = text.trim().to_ascii_lowercase();
    let mut modifiers = KeyModifiers::NONE;
    let mut rest = lower.as_str();
    loop {
        if let Some(r) = rest.strip_prefix("ctrl-") {
            modifiers |= KeyModifiers::CONTROL;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("alt-") {
            modifiers |= KeyModifiers::ALT;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("shift-").filter(|r| *r != "tab") {
            modifiers |= KeyModifiers::SHIFT;
            rest = r;
        } else {
            break;
        }
    }
    let code = match rest {
        "tab" => KeyCode::Tab,
        "shift-tab" => {
            modifiers |= KeyModifiers::SHIFT;
            KeyCode::BackTab
        }
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        f if f.starts_with('f') && f.len() > 1 => KeyCode::F(f[1..].parse().ok().filter(|n| (1..=12).contains(n))?),
        c if c.chars().count() == 1 => KeyCode::Char(c.chars().next()?),
        _ => return None,
    };
    // A BackTab arrives with SHIFT set by terminals; a plain char with SHIFT is the capital.
    Some(Chord { code, modifiers })
}

/// The user's key bindings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    bindings: HashMap<Chord, UserAction>,
}

impl Keymap {
    /// Read `text`; the problems are one line each, naming the line.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut map = Self::default();
        let mut problems = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parsed = line.split_once('=').and_then(|(k, a)| Some((parse_chord(k)?, UserAction::parse(a.trim())?)));
            match parsed {
                Some((chord, action)) => {
                    map.bindings.insert(chord, action);
                }
                None => problems.push(format!("keybindings line {}: cannot read `{line}` (use `ctrl-o = clear`; actions: clear, cycle-mode, quit)", i + 1)),
            }
        }
        (map, problems)
    }

    /// The action this key press is bound to.
    pub fn action(&self, k: &KeyEvent) -> Option<UserAction> {
        // Shift on a letter only makes the capital, which the character already says.
        let modifiers = if matches!(k.code, KeyCode::Char(_)) { k.modifiers - KeyModifiers::SHIFT } else { k.modifiers };
        let code = match k.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        };
        self.bindings.get(&Chord { code, modifiers }).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// Where the file lives: `JEV_KEYBINDINGS`, else `$XDG_CONFIG_HOME/jev-router/keybindings`,
/// else `~/.config/jev-router/keybindings`.
pub fn keymap_path(over: Option<String>, xdg: Option<String>, home: Option<String>) -> Option<PathBuf> {
    let non_empty = |v: Option<String>| v.filter(|s| !s.is_empty());
    if let Some(p) = non_empty(over) {
        return Some(PathBuf::from(p));
    }
    let base = non_empty(xdg).map(PathBuf::from).or_else(|| non_empty(home).map(|h| Path::new(&h).join(".config")))?;
    Some(base.join("jev-router").join("keybindings"))
}

/// The user's keymap from disk, empty when there is no file, with the problems found.
pub fn load() -> (Keymap, Vec<String>) {
    let var = |k: &str| std::env::var(k).ok();
    let Some(path) = keymap_path(var("JEV_KEYBINDINGS"), var("XDG_CONFIG_HOME"), var("HOME")) else { return Default::default() };
    match std::fs::read_to_string(&path) {
        Ok(text) => Keymap::parse(&text),
        Err(_) => Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    #[test]
    fn lines_bind_keys_to_actions_and_bad_lines_are_reported() {
        let (map, problems) = Keymap::parse("# mine\n\nctrl-o = clear\nf2 = cycle-mode\nalt-q=quit\nctrl-x = fly\nnonsense\nf13 = quit\n");
        assert_eq!(map.action(&press(KeyCode::Char('o'), KeyModifiers::CONTROL)), Some(UserAction::Clear));
        assert_eq!(map.action(&press(KeyCode::F(2), KeyModifiers::NONE)), Some(UserAction::CycleMode));
        assert_eq!(map.action(&press(KeyCode::Char('q'), KeyModifiers::ALT)), Some(UserAction::Quit));
        assert_eq!(map.action(&press(KeyCode::Char('o'), KeyModifiers::NONE)), None);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("line 6") || problems[0].contains("line 5"), "{problems:?}");
    }

    #[test]
    fn shift_tab_and_capitals_match_how_terminals_send_them() {
        let (map, _) = Keymap::parse("shift-tab = quit\nctrl-g = clear\n");
        assert_eq!(map.action(&press(KeyCode::BackTab, KeyModifiers::SHIFT)), Some(UserAction::Quit));
        assert_eq!(map.action(&press(KeyCode::Char('G'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)), Some(UserAction::Clear));
        assert!(Keymap::default().is_empty());
    }

    #[test]
    fn the_path_prefers_the_override_then_xdg_then_home() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(keymap_path(s("/k"), s("/x"), s("/h")), Some(PathBuf::from("/k")));
        assert_eq!(keymap_path(None, s("/x"), s("/h")), Some(PathBuf::from("/x/jev-router/keybindings")));
        assert_eq!(keymap_path(None, s(""), s("/h")), Some(PathBuf::from("/h/.config/jev-router/keybindings")));
        assert_eq!(keymap_path(None, None, None), None);
    }
}
