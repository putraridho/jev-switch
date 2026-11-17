//! `/terminal-setup`: make Shift-Enter insert a new line. Most terminals send the same
//! byte for Shift-Enter as for Enter, so the line editor cannot tell them apart. The fix
//! Claude Code uses works here too: have the terminal send Esc + Enter for Shift-Enter,
//! which jev-code already reads as a new line (the same as Alt-Enter).
//!
//! Which terminal this is comes from its environment variables; the snippet and the file
//! it goes in are per terminal. Installing appends the snippet (after a backup) only when
//! the file does not already have it.

use std::path::{Path, PathBuf};

/// A terminal jev-code knows how to set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminal {
    Kitty,
    Ghostty,
    Alacritty,
    Foot,
    WezTerm,
    VsCode,
}

/// What the environment says about the terminal, read once.
#[derive(Debug, Clone, Default)]
pub struct TermEnv {
    pub term: Option<String>,
    pub term_program: Option<String>,
    pub kitty_window: bool,
    pub alacritty_window: bool,
    pub ghostty_resources: bool,
    pub wezterm_pane: bool,
}

impl TermEnv {
    pub fn from_process() -> Self {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Self {
            term: get("TERM"),
            term_program: get("TERM_PROGRAM"),
            kitty_window: get("KITTY_WINDOW_ID").is_some(),
            alacritty_window: get("ALACRITTY_WINDOW_ID").is_some() || get("ALACRITTY_SOCKET").is_some(),
            ghostty_resources: get("GHOSTTY_RESOURCES_DIR").is_some(),
            wezterm_pane: get("WEZTERM_PANE").is_some(),
        }
    }
}

/// The terminal this is, when it can be told.
pub fn detect(env: &TermEnv) -> Option<Terminal> {
    let program = env.term_program.as_deref().unwrap_or("").to_ascii_lowercase();
    let term = env.term.as_deref().unwrap_or("").to_ascii_lowercase();
    if env.kitty_window || term == "xterm-kitty" {
        Some(Terminal::Kitty)
    } else if env.ghostty_resources || program == "ghostty" || term == "xterm-ghostty" {
        Some(Terminal::Ghostty)
    } else if env.wezterm_pane || program == "wezterm" {
        Some(Terminal::WezTerm)
    } else if program == "vscode" {
        Some(Terminal::VsCode)
    } else if env.alacritty_window || term == "alacritty" {
        Some(Terminal::Alacritty)
    } else if term.starts_with("foot") {
        Some(Terminal::Foot)
    } else {
        None
    }
}

impl Terminal {
    pub fn name(self) -> &'static str {
        match self {
            Self::Kitty => "kitty",
            Self::Ghostty => "Ghostty",
            Self::Alacritty => "Alacritty",
            Self::Foot => "foot",
            Self::WezTerm => "WezTerm",
            Self::VsCode => "VS Code",
        }
    }

    /// The config file the snippet goes in, under `config_home` (`~/.config`).
    pub fn config_file(self, config_home: &Path) -> PathBuf {
        match self {
            Self::Kitty => config_home.join("kitty/kitty.conf"),
            Self::Ghostty => config_home.join("ghostty/config"),
            Self::Alacritty => config_home.join("alacritty/alacritty.toml"),
            Self::Foot => config_home.join("foot/foot.ini"),
            Self::WezTerm => config_home.join("wezterm/wezterm.lua"),
            Self::VsCode => config_home.join("Code/User/keybindings.json"),
        }
    }

    /// The lines that make Shift-Enter send Esc + Enter.
    pub fn snippet(self) -> &'static str {
        match self {
            Self::Kitty => "map shift+enter send_text all \\x1b\\r",
            Self::Ghostty => "keybind = shift+enter=text:\\x1b\\r",
            Self::Alacritty => "[[keyboard.bindings]]\nkey = \"Return\"\nmods = \"Shift\"\nchars = \"\\u001b\\r\"",
            Self::Foot => "[text-bindings]\n\\x1b\\x0d = Shift+Return",
            Self::WezTerm => "-- inside the table your config returns, in `keys = { ... }`:\n{ key = \"Enter\", mods = \"SHIFT\", action = wezterm.action.SendString(\"\\x1b\\r\") },",
            Self::VsCode => "{ \"key\": \"shift+enter\", \"command\": \"workbench.action.terminal.sendSequence\", \"args\": { \"text\": \"\\u001b\\r\" }, \"when\": \"terminalFocus\" }",
        }
    }

    /// Whether appending the snippet to the end of the file is valid there. WezTerm's Lua
    /// and VS Code's JSON need it placed inside a structure, by hand.
    pub fn can_append(self) -> bool {
        matches!(self, Self::Kitty | Self::Ghostty | Self::Alacritty | Self::Foot)
    }

    /// How the change takes effect.
    pub fn reload_hint(self) -> &'static str {
        match self {
            Self::Kitty => "reload with ctrl+shift+F5, or open a new window",
            Self::Ghostty => "reload the config (ctrl+shift+,) or open a new window",
            Self::Alacritty => "Alacritty reloads it by itself",
            Self::Foot => "open a new foot window",
            Self::WezTerm => "WezTerm reloads it by itself",
            Self::VsCode => "VS Code applies it at once",
        }
    }
}

/// What installing did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    /// The snippet was appended; the backup of the file as it was, if it existed.
    Added { backup: Option<PathBuf> },
    /// The file already has it.
    AlreadyThere,
}

/// Why installing did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// The snippet must go inside a structure; the person adds it by hand.
    ByHand,
    Io(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ByHand => f.write_str("this terminal's config needs the lines placed by hand"),
            Self::Io(e) => f.write_str(e),
        }
    }
}

/// Whether appending would break a config that already has the same table or section:
/// Alacritty's `bindings = [...]` (a second `[[keyboard.bindings]]` is a duplicate key),
/// foot's `[text-bindings]`.
fn would_clash(terminal: Terminal, text: &str) -> bool {
    let lines = || text.lines().map(str::trim).filter(|l| !l.starts_with('#'));
    match terminal {
        Terminal::Alacritty => lines().any(|l| l.split('=').next().is_some_and(|k| k.trim() == "bindings")),
        Terminal::Foot => lines().any(|l| l == "[text-bindings]"),
        _ => false,
    }
}

/// Append the snippet of `terminal` to `file`, after copying the file to `<file>.jev-bak`.
pub fn install(terminal: Terminal, file: &Path) -> Result<Installed, InstallError> {
    if !terminal.can_append() {
        return Err(InstallError::ByHand);
    }
    let io = |e: std::io::Error| InstallError::Io(format!("{}: {e}", file.display()));
    let existing = match std::fs::read_to_string(file) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io(e)),
    };
    let snippet = terminal.snippet();
    if existing.as_deref().is_some_and(|t| t.contains(snippet)) {
        return Ok(Installed::AlreadyThere);
    }
    if existing.as_deref().is_some_and(|t| would_clash(terminal, t)) {
        return Err(InstallError::ByHand);
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let backup = match &existing {
        Some(text) => {
            let path = PathBuf::from(format!("{}.jev-bak", file.display()));
            std::fs::write(&path, text).map_err(io)?;
            Some(path)
        }
        None => None,
    };
    let mut text = existing.unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("\n# Shift-Enter inserts a new line in jev-code (added by /terminal-setup)\n{snippet}\n"));
    std::fs::write(file, text).map_err(io)?;
    Ok(Installed::Added { backup })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminal_is_told_from_its_environment() {
        let env = |f: fn(&mut TermEnv)| {
            let mut e = TermEnv::default();
            f(&mut e);
            detect(&e)
        };
        assert_eq!(env(|e| e.kitty_window = true), Some(Terminal::Kitty));
        assert_eq!(env(|e| e.term = Some("xterm-ghostty".into())), Some(Terminal::Ghostty));
        assert_eq!(env(|e| e.alacritty_window = true), Some(Terminal::Alacritty));
        assert_eq!(env(|e| e.term = Some("foot-extra".into())), Some(Terminal::Foot));
        assert_eq!(env(|e| e.term_program = Some("vscode".into())), Some(Terminal::VsCode));
        assert_eq!(env(|e| e.term = Some("xterm-256color".into())), None);
    }

    #[test]
    fn installing_appends_once_and_keeps_a_backup() {
        let dir = std::env::temp_dir().join(format!("jev-termsetup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = Terminal::Kitty.config_file(&dir);
        assert_eq!(install(Terminal::Kitty, &file), Ok(Installed::Added { backup: None }), "a missing file is created");
        assert_eq!(install(Terminal::Kitty, &file), Ok(Installed::AlreadyThere));

        let alacritty = Terminal::Alacritty.config_file(&dir);
        std::fs::create_dir_all(alacritty.parent().unwrap()).unwrap();
        std::fs::write(&alacritty, "[font]\nsize = 11").unwrap();
        let Ok(Installed::Added { backup: Some(backup) }) = install(Terminal::Alacritty, &alacritty) else { panic!() };
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "[font]\nsize = 11");
        let text = std::fs::read_to_string(&alacritty).unwrap();
        assert!(text.starts_with("[font]\nsize = 11\n\n# Shift-Enter") && text.ends_with("chars = \"\\u001b\\r\"\n"), "{text}");

        assert_eq!(install(Terminal::WezTerm, &dir.join("w.lua")), Err(InstallError::ByHand));

        let inline = dir.join("inline.toml");
        std::fs::write(&inline, "[keyboard]\nbindings = [ { key = \"N\", mods = \"Control\", action = \"SpawnNewInstance\" } ]\n").unwrap();
        assert_eq!(install(Terminal::Alacritty, &inline), Err(InstallError::ByHand), "an inline bindings list is not broken");
        assert!(!std::fs::read_to_string(&inline).unwrap().contains("Shift-Enter"));
        let foot = dir.join("foot.ini");
        std::fs::write(&foot, "[text-bindings]\nx = Control+y\n").unwrap();
        assert_eq!(install(Terminal::Foot, &foot), Err(InstallError::ByHand));
        let _ = std::fs::remove_dir_all(dir);
    }
}
