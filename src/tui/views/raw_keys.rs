//! The terminal side of the pickers (`/resume`, `/mcp`): raw key input and a block of lines
//! redrawn in place. The pickers themselves are plain state; this is the only part that
//! touches the terminal.

use std::io::Write;

/// What one wait for keys gave.
pub enum Read {
    /// Bytes from the terminal: one escape sequence whole, or typed or pasted text.
    Bytes(Vec<u8>),
    /// Nothing arrived in time.
    Timeout,
    /// The terminal is gone (end of input or a read error).
    Closed,
}

/// The terminal in raw mode (no echo, no line buffering, no signal keys) while this lives,
/// with stdin locked away from the line editor. Dropping it puts everything back.
#[cfg(unix)]
pub struct RawKeys {
    saved: libc::termios,
    raw: libc::termios,
    _owner: std::sync::MutexGuard<'static, ()>,
}

#[cfg(unix)]
impl RawKeys {
    /// Raw mode on stdin, or `None` when stdin is not a terminal.
    pub fn enter() -> Option<Self> {
        let fd = libc::STDIN_FILENO;
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            return None;
        }
        let owner = crate::tui::input::line_editor::STDIN_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: termios is plain data; tcgetattr fills it or fails.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return None;
        }
        let mut raw = saved;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        // SAFETY: both calls only change the terminal's own state.
        unsafe {
            libc::tcflush(fd, libc::TCIFLUSH);
            libc::tcsetattr(fd, libc::TCSANOW, &raw);
        }
        Some(Self { saved, raw, _owner: owner })
    }

    /// Wait for keys, at most `timeout_ms` (`None`: as long as it takes). A lone Esc is a
    /// key and Esc followed by more is an arrow, so after a lone Esc it waits a tenth of a
    /// second for the rest.
    pub fn read(&self, timeout_ms: Option<i32>) -> Read {
        let fd = libc::STDIN_FILENO;
        if let Some(ms) = timeout_ms {
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: one live pollfd.
            match unsafe { libc::poll(&mut pfd, 1, ms) } {
                0 => return Read::Timeout,
                n if n < 0 => return if interrupted() { Read::Timeout } else { Read::Closed },
                _ => {}
            }
        }
        let mut buf = [0u8; 64];
        // SAFETY: reads at most buf.len() bytes into a live local buffer.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n == 0 {
            return Read::Closed;
        }
        if n < 0 {
            return if interrupted() { Read::Timeout } else { Read::Closed };
        }
        let mut bytes = buf[..n as usize].to_vec();
        if bytes == [0x1b] {
            let mut quick = self.raw;
            quick.c_cc[libc::VMIN] = 0;
            quick.c_cc[libc::VTIME] = 1;
            // SAFETY: as above, and the read stays inside the buffer.
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quick) };
            let more = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            // SAFETY: as above.
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &self.raw) };
            if more > 0 {
                bytes.extend_from_slice(&buf[..more as usize]);
            }
        }
        Read::Bytes(bytes)
    }
}

#[cfg(unix)]
impl Drop for RawKeys {
    fn drop(&mut self) {
        // SAFETY: puts back what tcgetattr read.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.saved) };
    }
}

#[cfg(unix)]
fn interrupted() -> bool {
    std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
}

/// Lines drawn below the cursor and redrawn in place; [`Block::clear`] takes them away.
#[derive(Default)]
pub struct Block {
    drawn: usize,
}

impl Block {
    /// Replace what was drawn with `lines`.
    pub fn draw(&mut self, lines: &[String]) {
        let mut out = std::io::stdout().lock();
        self.erase(&mut out);
        for line in lines {
            let _ = writeln!(out, "{line}");
        }
        let _ = out.flush();
        self.drawn = lines.len();
    }

    /// Take the lines off the screen, leaving the cursor where they started.
    pub fn clear(&mut self) {
        let mut out = std::io::stdout().lock();
        self.erase(&mut out);
        let _ = out.flush();
        self.drawn = 0;
    }

    fn erase(&self, out: &mut impl Write) {
        if self.drawn > 0 {
            let _ = write!(out, "\x1b[{}A", self.drawn);
        }
        let _ = write!(out, "\r\x1b[J");
    }
}
