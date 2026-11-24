//! The terminal as a shared resource: how wide it is, and putting it back the way we
//! found it. The live status line hides the cursor and the line editor switches the
//! terminal to raw mode, so every exit path (normal, error, panic) must undo both, or the
//! user is left with an invisible cursor and a shell that does not echo.
//!
//! The decisions are pure ([`width_from`], [`panic_texts`]); the few system calls sit in
//! thin wrappers beside them.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::render::show_cursor;

/// Set while the cursor is hidden, so [`restore`] only writes the escape when it must
/// (nothing is printed to a pipe or after a clean run).
static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);

/// Record that the cursor was hidden (`true`) or shown again (`false`).
pub fn note_cursor_hidden(hidden: bool) {
    CURSOR_HIDDEN.store(hidden, Ordering::SeqCst);
}

/// Set while the interactive front end has the keyboard in its own modes (bracketed paste,
/// the keyboard protocol that tells Shift-Enter from Enter), so [`restore`] turns them off.
static FRONT_KEYS: AtomicBool = AtomicBool::new(false);

/// Record that the front end switched its key modes on (`true`) or off (`false`).
pub fn note_front_keys(on: bool) {
    FRONT_KEYS.store(on, Ordering::SeqCst);
}

/// Set while the front end is on the alternate screen (its
/// full-screen layout), so [`restore`] leaves both.
static ALT_SCREEN: AtomicBool = AtomicBool::new(false);

/// Record that the alternate screen was entered (`true`) or left (`false`).
pub fn note_alt_screen(on: bool) {
    ALT_SCREEN.store(on, Ordering::SeqCst);
}

/// Columns the console may lay text out in, when the front end shows it in a pane narrower
/// than the terminal (the chat beside the side panel); 0 means the whole terminal.
static WIDTH_CAP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Set (or with `None` clear) the width the console lays text out in.
pub fn set_width_cap(cols: Option<usize>) {
    WIDTH_CAP.store(cols.unwrap_or(0), Ordering::SeqCst);
}

/// The cap set by [`set_width_cap`], if any.
pub fn width_cap() -> Option<usize> {
    Some(WIDTH_CAP.load(Ordering::SeqCst)).filter(|c| *c > 0)
}

/// Leave the alternate screen.
pub const LEAVE_ALT_SCREEN: &[u8] = b"\x1b[?1049l";

/// Set while the keyboard protocol is pushed (it is not asked for inside tmux, on a console
/// or on a dumb terminal, see [`crate::tty::terminal_environment`]), so [`restore`] only pops what was pushed.
static KEYBOARD_PROTOCOL: AtomicBool = AtomicBool::new(false);

/// Record that the keyboard protocol was pushed (`true`) or popped (`false`); returns whether
/// it was pushed before this call.
pub fn note_keyboard_protocol(on: bool) -> bool {
    KEYBOARD_PROTOCOL.swap(on, Ordering::SeqCst)
}

/// What puts the keyboard and the cursor back: pop the keyboard protocol (when it was pushed),
/// stop bracketed paste, show the cursor.
fn front_restore_bytes(keyboard_protocol: bool) -> &'static [u8] {
    if keyboard_protocol {
        b"\x1b[<u\x1b[?2004l\x1b[?25h"
    } else {
        b"\x1b[?2004l\x1b[?25h"
    }
}

/// The exit status a shell reports for a program ended by `signal`.
pub fn exit_code_for_signal(signal: i32) -> i32 {
    128 + signal
}

/// Put the terminal back and end the process when it is told to stop from outside: SIGTERM
/// (`kill`, a closing tmux pane's cleanup), SIGHUP (the terminal window or ssh connection
/// went away). Without this the default action ends the process at once and leaves raw mode,
/// bracketed paste and a hidden cursor behind in the user's shell. Ctrl-C is not handled
/// here: it belongs to the turn. Call once, inside the runtime.
#[cfg(unix)]
pub fn restore_on_termination() {
    use tokio::signal::unix::{signal, SignalKind};
    for (kind, number) in [(SignalKind::terminate(), libc::SIGTERM), (SignalKind::hangup(), libc::SIGHUP)] {
        let Ok(mut stream) = signal(kind) else { continue };
        tokio::spawn(async move {
            if stream.recv().await.is_some() {
                restore();
                std::process::exit(exit_code_for_signal(number));
            }
        });
    }
}

/// Windows has no SIGTERM; the console tells a program that its window is closing (or the
/// machine is shutting down) through these events.
#[cfg(windows)]
pub fn restore_on_termination() {
    use tokio::signal::windows::{ctrl_close, ctrl_logoff, ctrl_shutdown};
    macro_rules! watch {
        ($stream:expr) => {
            if let Ok(mut stream) = $stream {
                tokio::spawn(async move {
                    if stream.recv().await.is_some() {
                        restore();
                        std::process::exit(1);
                    }
                });
            }
        };
    }
    watch!(ctrl_close());
    watch!(ctrl_logoff());
    watch!(ctrl_shutdown());
}

#[cfg(unix)]
static SAVED_TERMIOS: std::sync::OnceLock<libc::termios> = std::sync::OnceLock::new();

/// Remembers the terminal settings at startup and restores them when dropped. Hold one
/// for the whole run: `let _terminal = TerminalGuard::install();`.
pub struct TerminalGuard(());

impl TerminalGuard {
    pub fn install() -> Self {
        #[cfg(unix)]
        {
            // SAFETY: `termios` is plain data; `tcgetattr` fills it and reports failure (not a
            // terminal) by returning -1, in which case nothing is saved.
            let mut t: libc::termios = unsafe { std::mem::zeroed() };
            if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut t) } == 0 {
                let _ = SAVED_TERMIOS.set(t);
            }
        }
        Self(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}

/// Put the terminal back: cursor visible, and the modes (echo, line buffering) it had at
/// startup. Safe to call more than once and from a panic hook; does nothing when there is
/// nothing to undo.
pub fn restore() {
    use std::io::Write;
    if ALT_SCREEN.swap(false, Ordering::SeqCst) {
        let mut out = std::io::stdout();
        let _ = out.write_all(LEAVE_ALT_SCREEN);
        let _ = out.flush();
    }
    if FRONT_KEYS.swap(false, Ordering::SeqCst) {
        let mut out = std::io::stdout();
        let _ = out.write_all(front_restore_bytes(KEYBOARD_PROTOCOL.swap(false, Ordering::SeqCst)));
        let _ = out.flush();
    }
    if CURSOR_HIDDEN.swap(false, Ordering::SeqCst) {
        let mut err = std::io::stderr();
        let _ = err.write_all(show_cursor().as_bytes());
        let _ = err.flush();
    }
    #[cfg(unix)]
    if let Some(t) = SAVED_TERMIOS.get() {
        // SAFETY: `t` is the value `tcgetattr` produced for this descriptor at startup.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, t);
        }
    }
}

/// Terminal columns: the window size when the OS says (a real size, not 0), else the
/// `COLUMNS` variable (shells rarely export it, so it is only a fallback), else
/// [`DEFAULT_WIDTH`].
pub fn width() -> usize {
    width_from(ioctl_size().map(|(cols, _)| cols), std::env::var("COLUMNS").ok().as_deref())
}

/// What the console asks of the terminal while it draws, so tests can stand in for it:
/// how wide it is RIGHT NOW (a window can be resized mid-turn) and whether typed keys are
/// echoed back (they must not be while the status line owns the row).
pub trait Screen: Send {
    /// Live window columns, when the OS says; `None` leaves the caller its own fallback.
    fn columns(&self) -> Option<usize>;
    /// Whether typed keys are echoed right now; `None` when stdin is not a terminal (so there
    /// is nothing to switch and nothing to restore).
    fn echo(&self) -> Option<bool>;
    fn set_echo(&self, on: bool);
}

/// The process's real terminal.
pub struct StdScreen;

impl Screen for StdScreen {
    fn columns(&self) -> Option<usize> {
        ioctl_size().map(|(cols, _)| cols).filter(|c| *c > 0)
    }

    fn echo(&self) -> Option<bool> {
        stdin_echo()
    }

    fn set_echo(&self, on: bool) {
        set_stdin_echo(on);
    }
}

#[cfg(unix)]
fn stdin_echo() -> Option<bool> {
    // SAFETY: `termios` is plain data; `tcgetattr` fills it and fails (-1) when stdin is not
    // a terminal, in which case there is no state to report.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        (libc::tcgetattr(libc::STDIN_FILENO, &mut t) == 0).then_some(t.c_lflag & libc::ECHO != 0)
    }
}

#[cfg(not(unix))]
fn stdin_echo() -> Option<bool> {
    None
}

#[cfg(unix)]
fn set_stdin_echo(on: bool) {
    // SAFETY: `termios` is plain data; `tcgetattr` fills it and fails (-1) when stdin is not
    // a terminal, in which case nothing is changed.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(libc::STDIN_FILENO, &mut t) != 0 {
            return;
        }
        if on {
            t.c_lflag |= libc::ECHO;
        } else {
            t.c_lflag &= !libc::ECHO;
        }
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
    }
}

#[cfg(not(unix))]
fn set_stdin_echo(_on: bool) {}

/// Terminal rows, when the OS knows them; `LINES` is the fallback.
pub fn rows() -> Option<usize> {
    ioctl_size().map(|(_, rows)| rows).filter(|r| *r > 0).or_else(|| std::env::var("LINES").ok()?.trim().parse().ok().filter(|r| *r > 0))
}

/// The pure half of [`width`]: a reported size above 0 wins over the environment.
pub fn width_from(reported: Option<usize>, columns_env: Option<&str>) -> usize {
    reported.filter(|c| *c > 0).unwrap_or_else(|| crate::render::width_from_env(columns_env)).max(1)
}

#[cfg(unix)]
fn ioctl_size() -> Option<(usize, usize)> {
    // stderr first: that is where the status line is drawn.
    for fd in [libc::STDERR_FILENO, libc::STDOUT_FILENO, libc::STDIN_FILENO] {
        // SAFETY: `winsize` is plain data and `TIOCGWINSZ` only writes into it.
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 {
            return Some((ws.ws_col as usize, ws.ws_row as usize));
        }
    }
    None
}

/// The console's window size, from the console API.
#[cfg(windows)]
fn ioctl_size() -> Option<(usize, usize)> {
    crossterm::terminal::size().ok().map(|(cols, rows)| (usize::from(cols), usize::from(rows)))
}

/// What a panic tells the user and what it writes to the log. The user text is short and
/// has no Rust internals; the log line has the message and location for a bug report.
pub fn panic_texts(message: &str, location: &str, log_path: Option<&Path>) -> (String, String) {
    let where_to = match log_path {
        Some(p) => format!("Details were written to {}.", p.display()),
        None => "No log file could be written.".to_string(),
    };
    let user = format!(
        "\njev-code hit an internal error and had to stop. Files it already changed stay as they are (`git status` shows them).\n{where_to}\nIf it keeps happening, please report it with that file.\n"
    );
    (user, format!("panic at {location}: {message}"))
}

/// Install the panic hook: restore the terminal, tell the user briefly, log the details.
/// Replaces the default hook, which prints a Rust message and a backtrace hint.
pub fn install_panic_hook(log_path: Option<std::path::PathBuf>) {
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        restore();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown".to_string());
        let location = info.location().map_or_else(|| "unknown".to_string(), |l| format!("{}:{}", l.file(), l.line()));
        let (user, line) = panic_texts(&message, &location, log_path.as_deref());
        if let Some(path) = &log_path {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{} {line}", jev_shared::infrastructure::jsonl_log::now_ms());
            }
        }
        let _ = std::io::stderr().write_all(user.as_bytes());
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::DEFAULT_WIDTH;

    #[test]
    fn a_reported_window_size_beats_the_environment() {
        assert_eq!(width_from(Some(132), Some("80")), 132);
        assert_eq!(width_from(Some(0), Some("100")), 100, "a 0-wide window is 'unknown'");
        assert_eq!(width_from(None, Some("100")), 100);
        assert_eq!(width_from(None, None), DEFAULT_WIDTH);
        assert_eq!(width_from(None, Some("nope")), DEFAULT_WIDTH);
    }

    #[test]
    fn the_panic_text_is_short_and_has_no_rust_internals() {
        let (user, log) = panic_texts("index out of bounds", "src/x.rs:9", Some(Path::new("/s/panic.log")));
        assert!(user.contains("/s/panic.log") && user.contains("internal error"), "{user}");
        assert!(!user.contains("index out of bounds") && !user.contains("src/x.rs"), "{user}");
        assert_eq!(log, "panic at src/x.rs:9: index out of bounds");
        let (none, _) = panic_texts("x", "y", None);
        assert!(none.contains("No log file"), "{none}");
    }

    #[test]
    fn the_keyboard_protocol_is_only_popped_when_it_was_pushed() {
        assert!(front_restore_bytes(true).starts_with(b"\x1b[<u"));
        assert!(!front_restore_bytes(false).windows(4).any(|w| w == b"\x1b[<u"));
        for pushed in [true, false] {
            let bytes = String::from_utf8_lossy(front_restore_bytes(pushed)).into_owned();
            assert!(bytes.contains("\x1b[?2004l") && bytes.ends_with("\x1b[?25h"), "paste off and cursor back in both cases: {bytes:?}");
        }
    }

    #[test]
    fn a_signal_ends_the_program_with_the_shells_status() {
        assert_eq!(exit_code_for_signal(15), 143);
        assert_eq!(exit_code_for_signal(1), 129);
    }

    #[test]
    fn restore_is_quiet_when_nothing_was_changed() {
        // No cursor was hidden, so there is nothing to write and nothing to panic about.
        note_cursor_hidden(false);
        restore();
        restore();
    }
}
