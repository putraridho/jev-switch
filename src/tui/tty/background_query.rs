//! Is the terminal light or dark? Asked of the terminal itself (OSC 11, which most modern
//! terminals answer), because colours chosen for one background wash out on the other.
//! Best effort: no answer in a moment, inside tmux or screen, or off a terminal means
//! "unknown", and the caller keeps dark.

/// Light (`true`) or dark (`false`) from an OSC 11 reply such as
/// `\x1b]11;rgb:ffff/ffff/ffff\x07`: the colour's perceived brightness against the middle.
pub fn parse_osc11(reply: &[u8]) -> Option<bool> {
    let text = String::from_utf8_lossy(reply);
    let rest = text.split("rgb:").nth(1)?;
    let mut channels = rest.split('/').map(|part| {
        let hex: String = part.chars().take_while(|c| c.is_ascii_hexdigit()).take(4).collect();
        let max = 16f64.powi(hex.len() as i32) - 1.0;
        u32::from_str_radix(&hex, 16).ok().map(|v| f64::from(v) / max)
    });
    let (r, g, b) = (channels.next()??, channels.next()??, channels.next()??);
    Some(0.2126 * r + 0.7152 * g + 0.0722 * b > 0.5)
}

/// Ask the terminal. Raw mode must be on and nothing else reading the keyboard yet, or the
/// reply would be typed into the box.
#[cfg(unix)]
pub fn query_light() -> Option<bool> {
    use std::io::Write;
    use std::time::{Duration, Instant};
    let mut out = std::io::stdout();
    out.write_all(b"\x1b]11;?\x07").ok()?;
    out.flush().ok()?;
    let deadline = Instant::now() + Duration::from_millis(150);
    let mut reply = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut fd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd, and a buffer we own for the read.
        let ready = unsafe { libc::poll(&mut fd, 1, left.as_millis() as i32) };
        if ready <= 0 {
            break;
        }
        let mut buf = [0u8; 64];
        let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
        reply.extend_from_slice(&buf[..n as usize]);
        if reply.ends_with(b"\x07") || reply.ends_with(b"\x1b\\") {
            break;
        }
    }
    parse_osc11(&reply)
}

#[cfg(not(unix))]
pub fn query_light() -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_are_light_or_dark_by_brightness() {
        assert_eq!(parse_osc11(b"\x1b]11;rgb:ffff/ffff/ffff\x07"), Some(true));
        assert_eq!(parse_osc11(b"\x1b]11;rgb:0000/0000/0000\x1b\\"), Some(false));
        assert_eq!(parse_osc11(b"\x1b]11;rgb:1e/1e/2e\x07"), Some(false));
        assert_eq!(parse_osc11(b"\x1b]11;rgb:fdf6/e3e3/b6b6\x07"), Some(true));
        assert_eq!(parse_osc11(b""), None);
        assert_eq!(parse_osc11(b"junk"), None);
    }
}
