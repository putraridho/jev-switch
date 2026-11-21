//! The terminal: keyboard modes, size, and drawing a frame or finished output.

use std::time::Instant;

use crossterm::event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::{execute, terminal};

use crate::app::screen_state::App;
use crate::render::Style;
use crate::tty::frontend_screen::Screen;
use crate::views::frontend_view as view;

/// Switch on the keyboard modes the screen needs: raw keys, bracketed paste, and the
/// keyboard protocol that reports Shift-Enter apart from Enter (terminals that do not know it
/// ignore the request).
pub(super) fn keys_on() -> std::io::Result<()> {
    terminal::enable_raw_mode()?;
    pick_background();
    let mut out = std::io::stdout();
    if FULLSCREEN.load(std::sync::atomic::Ordering::SeqCst) {
        use std::io::Write;
        out.write_all(crate::tty::frontend_screen::FULL_ON.as_bytes())?;
        out.write_all(b"\x1b[2J")?;
        out.flush()?;
        crate::tty::terminal_guard::note_alt_screen(true);
    }
    execute!(out, crossterm::event::EnableBracketedPaste)?;
    // Inside tmux or on a console the request could be printed instead of understood.
    if term_env().keyboard_protocol() {
        execute!(out, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES))?;
        crate::tty::terminal_guard::note_keyboard_protocol(true);
    }
    crate::tty::terminal_guard::note_front_keys(true);
    Ok(())
}

/// Light or dark terminal: what `JEV_THEME` or `COLORFGBG` says, else what the terminal
/// answers (not asked inside tmux or screen, which may print the question instead).
fn pick_background() {
    let var = |k: &str| std::env::var(k).ok();
    let light = crate::render::background_from_env(var("JEV_THEME").as_deref(), var("COLORFGBG").as_deref()).or_else(|| {
        let env = term_env();
        (env.multiplexer.is_none() && env.kind == crate::tty::terminal_environment::Kind::Normal).then(crate::tty::background_query::query_light).flatten()
    });
    crate::render::set_light_background(light.unwrap_or(false));
}

pub(super) fn term_env() -> crate::tty::terminal_environment::TermEnv {
    crate::tty::terminal_environment::detect(&|name| std::env::var(name).ok())
}

/// Put the keyboard back as a command or the shell expects it.
pub(super) fn keys_off() {
    let mut out = std::io::stdout();
    if FULLSCREEN.load(std::sync::atomic::Ordering::SeqCst) {
        use std::io::Write;
        let _ = out.write_all(crate::tty::terminal_guard::LEAVE_ALT_SCREEN);
        let _ = out.flush();
        crate::tty::terminal_guard::note_alt_screen(false);
    }
    if crate::tty::terminal_guard::note_keyboard_protocol(false) {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(out, crossterm::event::DisableBracketedPaste);
    let _ = terminal::disable_raw_mode();
    crate::tty::terminal_guard::note_front_keys(false);
}

/// Turns the keyboard modes off when dropped, on every way out of the loop.
pub(super) struct KeyModes;

impl Drop for KeyModes {
    fn drop(&mut self) {
        keys_off();
    }
}

pub(super) fn width() -> usize {
    terminal::size().map(|(w, _)| usize::from(w)).unwrap_or_else(|_| crate::tty::terminal_guard::width())
}

/// The full-screen layout is on: the app keeps the conversation and draws the whole terminal
/// (alternate screen, chat pane, input box, full-height side panel).
pub(super) static FULLSCREEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Full screen on a terminal, unless `JEV_INLINE=1` asks for the conversation to go into the
/// terminal's own scrollback instead.
pub(super) fn fullscreen_wanted() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal() && std::env::var("JEV_INLINE").map_or(true, |v| v.is_empty() || v == "0")
}

/// Terminal columns and rows.
fn size() -> (usize, usize) {
    terminal::size().map(|(w, h)| (usize::from(w), usize::from(h))).unwrap_or_else(|_| (crate::tty::terminal_guard::width(), 24))
}

/// The columns text is laid out in: the chat pane when the side panel has its own column.
pub(super) fn text_width(app: &App) -> usize {
    let cols = size().0;
    if app.fullscreen {
        cols.saturating_sub(1).max(20)
    } else {
        cols
    }
}

/// What the screen shows now.
fn frame(app: &mut App, style: Style) -> view::Frame {
    let (cols, rows) = size();
    if app.fullscreen {
        crate::tty::terminal_guard::set_width_cap(Some(text_width(app)));
        let (frame, pane) = view::render_full(app, cols, rows, Instant::now(), style);
        app.pane_rows = pane;
        frame
    } else {
        view::render(app, cols, Instant::now(), style)
    }
}

pub(super) fn draw(screen: &mut Screen<std::io::Stdout>, app: &mut App, style: Style) {
    let f = frame(app, style);
    if app.fullscreen {
        screen.draw_full(&f);
    } else {
        screen.draw(&f);
    }
}

/// Finished output: into the conversation, or above the live area inline.
pub(super) fn show(screen: &mut Screen<std::io::Stdout>, app: &mut App, text: &str, style: Style) {
    if app.fullscreen {
        app.show(text);
    } else {
        let f = frame(app, style);
        screen.print_above(text, &f);
    }
}
