//! The clipboard: a program that takes text on stdin, else the terminal's OSC 52.

use std::io::Write;
use std::process::{Command, Stdio};

/// A program that takes text on stdin and puts it on the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardTool {
    WlCopy,
    Xclip,
    Xsel,
    Pbcopy,
}

impl ClipboardTool {
    fn program(self) -> &'static str {
        match self {
            Self::WlCopy => "wl-copy",
            Self::Xclip => "xclip",
            Self::Xsel => "xsel",
            Self::Pbcopy => "pbcopy",
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Self::WlCopy | Self::Pbcopy => &[],
            Self::Xclip => &["-selection", "clipboard"],
            Self::Xsel => &["--clipboard", "--input"],
        }
    }
}

/// The clipboard programs worth trying here, best first: Wayland's when a Wayland
/// display is set, X11's when an X display is, and macOS's.
pub fn clipboard_tools(wayland: bool, x11: bool, macos: bool) -> Vec<ClipboardTool> {
    let mut tools = Vec::new();
    if wayland {
        tools.push(ClipboardTool::WlCopy);
    }
    if x11 {
        tools.extend([ClipboardTool::Xclip, ClipboardTool::Xsel]);
    }
    if macos {
        tools.push(ClipboardTool::Pbcopy);
    }
    tools
}

/// How text reached the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copied {
    By(ClipboardTool),
    /// Through the terminal (OSC 52); it works when the terminal allows it.
    Terminal,
}

/// The OSC 52 sequence that asks the terminal to set its clipboard to `text`.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", jev_shared::domain::base64::encode(text.as_bytes()))
}

/// Put `text` on the clipboard: the first clipboard program that works, else OSC 52 on
/// stderr when it is a terminal. The error is for the user.
pub fn copy_to_clipboard(text: &str, stderr_tty: bool) -> Result<Copied, String> {
    let has = |key: &str| std::env::var_os(key).is_some_and(|v| !v.is_empty());
    let tools = clipboard_tools(has("WAYLAND_DISPLAY"), has("DISPLAY"), cfg!(target_os = "macos"));
    for tool in tools {
        let child = Command::new(tool.program()).args(tool.args()).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        let Ok(mut child) = child else { continue };
        let wrote = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
        if wrote && child.wait().is_ok_and(|s| s.success()) {
            return Ok(Copied::By(tool));
        }
    }
    if stderr_tty {
        let mut err = std::io::stderr();
        let _ = err.write_all(osc52(text).as_bytes());
        let _ = err.flush();
        return Ok(Copied::Terminal);
    }
    Err("no clipboard program found (install wl-clipboard or xclip)".to_string())
}

impl std::fmt::Display for ClipboardTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.program())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_programs_follow_the_display() {
        use ClipboardTool::*;
        assert_eq!(clipboard_tools(true, true, false), [WlCopy, Xclip, Xsel]);
        assert_eq!(clipboard_tools(false, false, true), [Pbcopy]);
        assert!(clipboard_tools(false, false, false).is_empty());
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }
}
