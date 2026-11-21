//! The permission mode of a session, like Claude Code's: how much a turn may do without
//! asking. One value is shared by the line editor (Shift-Tab cycles it), `/mode`, the
//! engine (which passes it to every turn) and the approver (which answers in [`Mode::Auto`]).
//!
//! [`Mode::Auto`] approves tool requests without prompting or saving permission rules.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Ask before every edit and command (`default`).
    Ask,
    /// Edits are accepted, commands are asked (`acceptEdits`): what jev-code always did.
    #[default]
    Edit,
    /// Read and plan only; the model proposes a plan and changes nothing (`plan`).
    Plan,
    /// Edits and tool requests are accepted without prompting.
    Auto,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Ask, Mode::Edit, Mode::Plan, Mode::Auto];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Ask => "ask",
            Mode::Edit => "edit",
            Mode::Plan => "plan",
            Mode::Auto => "auto",
        }
    }

    /// What this mode does, for `/mode` and the empty-line hint.
    pub fn about(self) -> &'static str {
        match self {
            Mode::Ask => "asks before every edit and command",
            Mode::Edit => "accepts edits, asks before commands",
            Mode::Plan => "plans only, changes nothing",
            Mode::Auto => "accepts edits and tool requests without asking",
        }
    }

    /// The Claude Code `--permission-mode` for this mode. `Auto` runs as `acceptEdits`: the
    /// extra answers come from jev-code, not from a looser Claude Code mode.
    pub fn claude_flag(self) -> &'static str {
        match self {
            Mode::Ask => "default",
            Mode::Edit | Mode::Auto => "acceptEdits",
            Mode::Plan => "plan",
        }
    }

    /// The next mode in Shift-Tab order: ask, edit, plan, auto, and round again.
    pub fn next(self) -> Mode {
        match self {
            Mode::Ask => Mode::Edit,
            Mode::Edit => Mode::Plan,
            Mode::Plan => Mode::Auto,
            Mode::Auto => Mode::Ask,
        }
    }

    fn code(self) -> u8 {
        Mode::ALL.iter().position(|m| *m == self).unwrap_or(1) as u8
    }

    fn from_code(code: u8) -> Mode {
        Mode::ALL.get(usize::from(code)).copied().unwrap_or_default()
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A word that names no mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownMode(pub String);

impl fmt::Display for UnknownMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown mode '{}': use ask, edit, plan or auto", self.0)
    }
}

impl FromStr for Mode {
    type Err = UnknownMode;

    /// Accepts jev-code's names and Claude Code's (`default`, `acceptEdits`).
    fn from_str(s: &str) -> Result<Self, UnknownMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ask" | "default" => Ok(Mode::Ask),
            "edit" | "edits" | "acceptedits" | "accept-edits" => Ok(Mode::Edit),
            "plan" => Ok(Mode::Plan),
            "auto" => Ok(Mode::Auto),
            other => Err(UnknownMode(other.to_string())),
        }
    }
}

/// The session's mode where the editor's key handler, the REPL and the engine all see it.
#[derive(Debug, Clone, Default)]
pub struct SharedMode(Arc<AtomicU8>);

impl SharedMode {
    pub fn new(mode: Mode) -> Self {
        Self(Arc::new(AtomicU8::new(mode.code())))
    }

    pub fn get(&self) -> Mode {
        Mode::from_code(self.0.load(Ordering::Relaxed))
    }

    pub fn set(&self, mode: Mode) {
        self.0.store(mode.code(), Ordering::Relaxed);
    }

    /// Move to the next mode and return it.
    pub fn cycle(&self) -> Mode {
        let next = self.get().next();
        self.set(next);
        next
    }
}

/// The mode a session starts in: `CODE_PERMISSION_MODE` when it names one (Claude Code's
/// names work), else [`Mode::Edit`]. `bypassPermissions` and the like are not modes here.
pub fn initial(env: Option<&str>) -> Mode {
    env.and_then(|v| v.parse().ok()).unwrap_or_default()
}

/// The file in the state folder that keeps the mode between sessions.
pub fn saved_path(state_dir: &Path) -> PathBuf {
    state_dir.join("mode")
}

/// The mode an interactive session starts in: what was asked on the command line, else what
/// the last session left (`/mode`, Shift-Tab), else [`initial`].
pub fn start(flag: Option<Mode>, saved: Option<Mode>, env: Option<&str>) -> Mode {
    flag.or(saved).unwrap_or_else(|| initial(env))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse_in_both_vocabularies_and_round_trip() {
        for m in Mode::ALL {
            assert_eq!(m.name().parse::<Mode>(), Ok(m));
            assert_eq!(Mode::from_code(m.code()), m);
        }
        assert_eq!("acceptEdits".parse::<Mode>(), Ok(Mode::Edit));
        assert_eq!("DEFAULT".parse::<Mode>(), Ok(Mode::Ask));
        assert!("bypassPermissions".parse::<Mode>().is_err());
        assert!("yolo".parse::<Mode>().unwrap_err().to_string().contains("ask, edit, plan or auto"));
    }

    #[test]
    fn cycling_visits_every_mode_and_comes_back() {
        let shared = SharedMode::new(Mode::Ask);
        let seen: Vec<Mode> = (0..4).map(|_| shared.cycle()).collect();
        assert_eq!(seen, [Mode::Edit, Mode::Plan, Mode::Auto, Mode::Ask]);
    }

    #[test]
    fn no_mode_maps_to_an_approve_everything_flag() {
        for m in Mode::ALL {
            assert!(matches!(m.claude_flag(), "default" | "acceptEdits" | "plan"), "{m}");
        }
        assert_eq!(Mode::Auto.claude_flag(), "acceptEdits");
    }

    #[test]
    fn the_start_mode_comes_from_the_environment_or_is_edit() {
        assert_eq!(initial(None), Mode::Edit);
        assert_eq!(initial(Some("plan")), Mode::Plan);
        assert_eq!(initial(Some("bypassPermissions")), Mode::Edit);
    }

    #[test]
    fn the_flag_beats_the_kept_mode_which_beats_the_environment() {
        assert_eq!(start(Some(Mode::Plan), Some(Mode::Auto), Some("ask")), Mode::Plan);
        assert_eq!(start(None, Some(Mode::Auto), Some("ask")), Mode::Auto);
        assert_eq!(start(None, None, Some("ask")), Mode::Ask);
        assert_eq!(start(None, None, None), Mode::Edit);
    }
}
