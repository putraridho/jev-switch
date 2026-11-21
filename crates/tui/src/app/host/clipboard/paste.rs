//! Read the desktop clipboard explicitly: terminals only paste text themselves.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const MAX_BYTES: u64 = 20 * 1024 * 1024;

async fn read(program: &str, args: &[&str]) -> Option<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?.take(MAX_BYTES + 1);
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.ok()?;
        if bytes.len() as u64 > MAX_BYTES {
            return None;
        }
        child.wait().await.ok()?.success().then_some(bytes)
    })
    .await
    .ok()
    .flatten();
    if result.is_none() {
        let _ = child.kill().await;
    }
    result
}

/// Ctrl-V reads a PNG, file-manager selection or text on Wayland and X11.
/// Saved images stay available to resumed turns, with owner-only permissions.
pub async fn paste_from_clipboard(state: Option<&Path>) -> Result<String, String> {
    let has = |key| std::env::var_os(key).is_some_and(|value| !value.is_empty());
    let mut sources = Vec::new();
    if has("WAYLAND_DISPLAY") {
        sources.push((
            "wl-paste",
            vec!["--list-types"],
            vec!["--no-newline", "--type"],
        ));
    }
    if has("DISPLAY") {
        sources.push((
            "xclip",
            vec!["-selection", "clipboard", "-o", "-t", "TARGETS"],
            vec!["-selection", "clipboard", "-o", "-t"],
        ));
    }
    for (program, list_args, args) in sources {
        let Some(types) = read(program, &list_args).await else {
            continue;
        };
        let types = String::from_utf8_lossy(&types);
        for mime in [
            "image/png",
            "text/uri-list",
            "text/plain;charset=utf-8",
            "UTF8_STRING",
            "text/plain",
        ] {
            if !types.lines().any(|line| line.trim() == mime) {
                continue;
            }
            let mut args = args.clone();
            args.push(mime);
            let Some(bytes) = read(program, &args).await else {
                continue;
            };
            if mime == "image/png" {
                return save_image(&bytes, state);
            }
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            let text = if mime == "text/uri-list" {
                format!(" {} ", uri_paths(&text))
            } else {
                text
            };
            let pasted = crate::tty::terminal_environment::sanitize_paste(&text);
            return Ok(pasted.text);
        }
    }
    Err("Cannot read the clipboard; on Linux install wl-clipboard (Wayland) or xclip (X11). You can also drag a file into the prompt.".into())
}

fn uri_paths(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ")
}

fn save_image(bytes: &[u8], state: Option<&Path>) -> Result<String, String> {
    use std::io::Write;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The clipboard did not contain a valid PNG image.".into());
    }
    let dir = state
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir)
        .join("clipboard");
    let save = || -> std::io::Result<String> {
        std::fs::create_dir_all(&dir)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = dir.join(format!("paste-{}-{stamp}.png", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(bytes)?;
        // A file URL handles spaces and quotes without depending on shell quoting.
        let path = path.canonicalize()?;
        let encoded = path
            .to_string_lossy()
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
                    char::from(b).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>();
        Ok(format!(" file://{encoded} "))
    };
    save().map_err(|error| format!("Cannot save the clipboard image: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_image_becomes_an_attachment_and_is_private() {
        let dir = std::env::temp_dir().join(format!("jev-clipboard-test-{}", std::process::id()));
        let png = b"\x89PNG\r\n\x1a\nfixture";
        let text = save_image(png, Some(&dir)).unwrap();
        let attached = jev_turn::application::attach::extract(&text, &dir);
        assert_eq!(attached.images.len(), 1);
        assert_eq!(std::fs::read(&attached.images[0]).unwrap(), png);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&attached.images[0])
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert!(save_image(b"not png", Some(&dir)).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn clipboard_process_failure_and_timeout_return_without_hanging() {
        assert_eq!(
            read("sh", &["-c", "printf clipboard"]).await,
            Some(b"clipboard".to_vec())
        );
        assert_eq!(read("sh", &["-c", "exit 1"]).await, None);
        assert_eq!(read("sh", &["-c", "exec sleep 10"]).await, None);
    }

    #[test]
    fn file_manager_selection_skips_comments_and_empty_lines() {
        assert_eq!(
            uri_paths("# files\r\nfile:///tmp/a%20b.png\r\n\r\nfile:///tmp/a.svg\r\n"),
            "file:///tmp/a%20b.png file:///tmp/a.svg"
        );
    }
}
