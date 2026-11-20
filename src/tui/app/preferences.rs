//! Choices that carry over to the next `jev-code`: what the person set with `/mode`,
//! `/orchestrate` and `/provider` (or Shift-Tab) stays set. One small file each in the state
//! folder, replaced whole so a crash never leaves half a word; a missing or unreadable file
//! is no choice at all, and the environment or the default decides.

use std::path::Path;

use crate::approval::domain::mode;
use crate::approval::domain::mode::Mode;

/// An on/off setting that is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    /// `/orchestrate on|off`.
    Orchestrate,
    /// `/provider auto|off`: whether the router picks among providers.
    Routing,
    /// `/thinking on|off`: print the model's whole reasoning.
    Thinking,
}

impl Flag {
    fn file(self) -> &'static str {
        match self {
            Flag::Orchestrate => "orchestrate",
            Flag::Routing => "routing",
            Flag::Thinking => "thinking",
        }
    }
}

/// The kept value of `flag`, when there is one.
pub fn load_flag(state_dir: &Path, flag: Flag) -> Option<bool> {
    match std::fs::read_to_string(state_dir.join(flag.file())).ok()?.trim() {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

/// Keep `flag` for the next session.
pub fn save_flag(state_dir: &Path, flag: Flag, on: bool) -> std::io::Result<()> {
    crate::session::domain::catch_up::write_atomic(&state_dir.join(flag.file()), if on { "on" } else { "off" })
}

/// Everything the last session left, read once at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Saved {
    pub mode: Option<Mode>,
    pub orchestrate: Option<bool>,
    pub routing: Option<bool>,
    pub thinking: Option<bool>,
}

impl Saved {
    pub fn load(state_dir: &Path) -> Self {
        Self {
            mode: mode::load_saved(state_dir),
            orchestrate: load_flag(state_dir, Flag::Orchestrate),
            routing: load_flag(state_dir, Flag::Routing),
            thinking: load_flag(state_dir, Flag::Thinking),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("jev-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_kept_flag_survives_and_a_bad_file_is_no_choice() {
        let d = dir("flags");
        for flag in [Flag::Orchestrate, Flag::Routing, Flag::Thinking] {
            assert_eq!(load_flag(&d, flag), None);
            save_flag(&d, flag, false).unwrap();
            assert_eq!(load_flag(&d, flag), Some(false));
            save_flag(&d, flag, true).unwrap();
            assert_eq!(load_flag(&d, flag), Some(true));
        }
        // The flags do not share a file.
        save_flag(&d, Flag::Routing, false).unwrap();
        save_flag(&d, Flag::Thinking, false).unwrap();
        assert_eq!(load_flag(&d, Flag::Orchestrate), Some(true));
        assert_eq!(load_flag(&d, Flag::Thinking), Some(false));
        std::fs::write(d.join("orchestrate"), "maybe").unwrap();
        assert_eq!(load_flag(&d, Flag::Orchestrate), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn everything_the_last_session_left_is_read_together() {
        let d = dir("saved");
        assert_eq!(Saved::load(&d), Saved::default());
        mode::save(&d, Mode::Auto).unwrap();
        save_flag(&d, Flag::Orchestrate, false).unwrap();
        save_flag(&d, Flag::Thinking, true).unwrap();
        assert_eq!(Saved::load(&d), Saved { mode: Some(Mode::Auto), orchestrate: Some(false), routing: None, thinking: Some(true) });
        let _ = std::fs::remove_dir_all(&d);
    }
}
