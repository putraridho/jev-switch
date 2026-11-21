//! Vim input mode for the box (`JEV_VIM=1`): Esc leaves typing for normal mode, where
//! `h j k l w b 0 $ x dd D i a I A o` act on the text; `i`, `a`, `I`, `A`, `o` go back to
//! typing. Enter still sends and Ctrl chords keep their meaning, in both modes.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::input::editor::Editor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMode {
    Insert,
    Normal,
}

/// What the key did, so the caller knows whether to handle it as usual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Not vim's: the caller handles the key as it would without vim mode.
    Pass,
    /// Vim took it and the text did not change.
    Moved,
    /// Vim took it and the text changed.
    Edited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vim {
    pub mode: VimMode,
    /// A `d` waits for a second one.
    pending_d: bool,
}

impl Default for Vim {
    fn default() -> Self {
        Self { mode: VimMode::Insert, pending_d: false }
    }
}

/// Vim mode is on when `JEV_VIM` is set to anything but empty, `0`, `false` or `no`.
pub fn enabled_from(env: Option<&str>) -> bool {
    env.is_some_and(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no"))
}

impl Vim {
    /// The mode's label for the footer, `None` while typing.
    pub fn label(&self) -> Option<&'static str> {
        (self.mode == VimMode::Normal).then_some("NORMAL")
    }

    /// Handle one key. `busy` is a turn running: Esc then stops it and is never vim's.
    pub fn on_key(&mut self, ed: &mut Editor, k: &KeyEvent, busy: bool) -> Outcome {
        if k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return Outcome::Pass;
        }
        match (self.mode, k.code) {
            (_, KeyCode::Esc) if busy => Outcome::Pass,
            (VimMode::Insert, KeyCode::Esc) => {
                self.mode = VimMode::Normal;
                ed.left_in_line();
                Outcome::Moved
            }
            (VimMode::Insert, _) => Outcome::Pass,
            (VimMode::Normal, KeyCode::Esc) => {
                self.pending_d = false;
                Outcome::Moved
            }
            (VimMode::Normal, KeyCode::Char(c)) => self.normal(ed, c),
            (VimMode::Normal, _) => Outcome::Pass,
        }
    }

    fn insert(&mut self) -> Outcome {
        self.mode = VimMode::Insert;
        Outcome::Moved
    }

    fn normal(&mut self, ed: &mut Editor, c: char) -> Outcome {
        let was_d = std::mem::take(&mut self.pending_d);
        match c {
            'd' if was_d => {
                ed.delete_line();
                return Outcome::Edited;
            }
            'd' => self.pending_d = true,
            'h' => ed.left_in_line(),
            'l' => ed.right_in_line(),
            'j' => {
                ed.down();
            }
            'k' => {
                ed.up();
            }
            'w' => ed.word_right(),
            'b' => ed.word_left(),
            '0' => ed.home(),
            '$' => {
                ed.end();
                ed.left_in_line();
            }
            'x' => {
                ed.delete();
                return Outcome::Edited;
            }
            'D' => {
                ed.kill_to_end();
                return Outcome::Edited;
            }
            'i' => return self.insert(),
            'a' => {
                ed.right();
                return self.insert();
            }
            'I' => {
                ed.home();
                return self.insert();
            }
            'A' => {
                ed.end();
                return self.insert();
            }
            'o' => {
                ed.end();
                ed.insert_char('\n');
                self.insert();
                return Outcome::Edited;
            }
            _ => {}
        }
        Outcome::Moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn feed(v: &mut Vim, ed: &mut Editor, keys: &str) {
        for c in keys.chars() {
            let k = if c == '\x1b' { key(KeyCode::Esc) } else { key(KeyCode::Char(c)) };
            if v.on_key(ed, &k, false) == Outcome::Pass {
                ed.insert_char(c);
            }
        }
    }

    #[test]
    fn typing_goes_through_until_esc_then_keys_are_commands() {
        let (mut v, mut ed) = (Vim::default(), Editor::default());
        feed(&mut v, &mut ed, "hello world");
        assert_eq!(ed.text(), "hello world");
        feed(&mut v, &mut ed, "\x1b");
        assert_eq!(v.label(), Some("NORMAL"));
        feed(&mut v, &mut ed, "0xw");
        assert_eq!(ed.text(), "ello world");
        feed(&mut v, &mut ed, "iX");
        assert_eq!(ed.text(), "elloX world", "w moves to the end of the word, as the box's Alt-f does");
        assert_eq!(v.label(), None);
    }

    #[test]
    fn a_dd_removes_the_line_and_o_opens_one_below() {
        let (mut v, mut ed) = (Vim::default(), Editor::default());
        ed.set("one\ntwo\nthree");
        feed(&mut v, &mut ed, "\x1bkdd");
        assert_eq!(ed.text(), "one\nthree");
        feed(&mut v, &mut ed, "ofour");
        assert_eq!(ed.text(), "one\nthree\nfour");
        feed(&mut v, &mut ed, "\x1bkD");
        assert_eq!(ed.text(), "one\nthr\nfour");
        let mut one = Editor::default();
        one.set("only");
        feed(&mut Vim { mode: VimMode::Normal, ..Vim::default() }, &mut one, "dd");
        assert_eq!(one.text(), "");
    }

    #[test]
    fn append_motion_keys_and_pass_through_keys() {
        let (mut v, mut ed) = (Vim::default(), Editor::default());
        ed.set("ab cd");
        feed(&mut v, &mut ed, "\x1b0Axy");
        assert_eq!(ed.text(), "ab cdxy");
        feed(&mut v, &mut ed, "\x1bbhIz");
        assert!(ed.text().starts_with('z'), "{}", ed.text());
        let mut n = Vim { mode: VimMode::Normal, ..Vim::default() };
        assert_eq!(n.on_key(&mut ed, &key(KeyCode::Enter), false), Outcome::Pass, "Enter still sends");
        assert_eq!(n.on_key(&mut ed, &KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL), false), Outcome::Pass);
        assert_eq!(n.on_key(&mut ed, &key(KeyCode::Esc), true), Outcome::Pass, "Esc stops a running turn");
        assert_eq!(n.on_key(&mut ed, &key(KeyCode::Char('q')), false), Outcome::Moved, "unknown keys are ignored, not typed");
    }

    #[test]
    fn the_environment_switch() {
        assert!(enabled_from(Some("1")) && !enabled_from(Some("0")) && !enabled_from(None) && !enabled_from(Some("")));
    }
}
