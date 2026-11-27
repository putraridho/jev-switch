//! Files going with a chat message: what may be attached, how pasted content is kept, and how
//! the attachments are handed to the engine.
//!
//! Every attachment ends up as a file on disk. Pictures go to the model as images (the engine
//! takes their paths); anything else readable (text, code, PDFs) is listed by path in the
//! prompt for the model to open. Pasted content is written to a private staging folder first.

use jev_chat_core::ipc::{ChatAttachResult, ChatAttachment, ChatAttachmentKind};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The largest file that can be attached, in bytes (20 MB).
pub const MAX_BYTES: u64 = 20 * 1024 * 1024;

/// The most attachments one message can carry.
pub const MAX_PER_MESSAGE: usize = 10;

/// How long staged (pasted) files are kept before a later paste clears them.
const STAGE_KEEP: Duration = Duration::from_hours(7 * 24);

/// Extensions sent as images.
const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// Readable binary formats the model opens from a path.
const DOCUMENT_EXTENSIONS: [&str; 1] = ["pdf"];

/// How much of a file is read to tell text from binary.
const SNIFF_BYTES: u64 = 8 * 1024;

/// The prompt used when a message is only attachments.
const ONLY_ATTACHMENTS: &str = "See the attached files.";

/// Checks the file at `path` and describes it as an attachment, or says why it can't be one.
pub fn check_path(path: &Path) -> ChatAttachResult {
    check(path, display_name(path), false)
}

/// Writes pasted or uploaded bytes (`data`, base64) under `dir` as `name` and checks the
/// result. Old staged files are cleared on the way.
pub fn stage_data(dir: &Path, name: &str, data: &str, now: SystemTime) -> ChatAttachResult {
    let shown = safe_name(name);
    let Some(bytes) = decode_base64(data) else {
        return refused(&shown, None, "couldn't read the file");
    };
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > MAX_BYTES {
        return refused(&shown, Some(size), "over 20 MB");
    }
    match write_staged(dir, &shown, &bytes, now) {
        Some(path) => check(&path, shown, false),
        None => refused(&shown, Some(size), "couldn't keep the file"),
    }
}

/// Keeps pasted `text` under `dir` as a numbered text file and describes it.
pub fn stage_text(dir: &Path, text: &str, now: SystemTime) -> ChatAttachResult {
    let stamp = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let name = format!("pasted-text-{stamp}.txt");
    let size = u64::try_from(text.len()).unwrap_or(u64::MAX);
    if size > MAX_BYTES {
        return refused("Pasted text", Some(size), "over 20 MB");
    }
    match write_staged(dir, &name, text.as_bytes(), now) {
        Some(path) => check(&path, String::from("Pasted text"), true),
        None => refused("Pasted text", Some(size), "couldn't keep the text"),
    }
}

/// The prompt to run and the images to hand the engine for `prompt` with `attachments`.
/// Images are marked `[image: path]` in the text (as the terminal does) and passed by path;
/// other files are listed for the model to read.
pub fn compose(prompt: &str, attachments: &[ChatAttachment]) -> (String, Vec<PathBuf>) {
    let mut text = if prompt.trim().is_empty() && !attachments.is_empty() {
        String::from(ONLY_ATTACHMENTS)
    } else {
        prompt.trim().to_owned()
    };
    let images: Vec<PathBuf> = attachments
        .iter()
        .filter(|a| a.kind == ChatAttachmentKind::Image)
        .map(|a| PathBuf::from(&a.path))
        .collect();
    let files: Vec<&ChatAttachment> = attachments
        .iter()
        .filter(|a| a.kind != ChatAttachmentKind::Image)
        .collect();
    if !images.is_empty() {
        text.push_str("\n\n");
        let marks: Vec<String> = images
            .iter()
            .map(|p| format!("[image: {}]", p.display()))
            .collect();
        text.push_str(&marks.join("\n"));
    }
    if !files.is_empty() {
        text.push_str("\n\nAttached files (read them at these paths):");
        for file in files {
            let note = if file.kind == ChatAttachmentKind::Text {
                " (pasted text)"
            } else {
                ""
            };
            text.push_str("\n- ");
            text.push_str(&file.path);
            text.push_str(note);
        }
    }
    (text, images)
}

/// The private folder pasted files are kept in, under the engine's state folder.
pub fn staging_dir(state: &Path) -> PathBuf {
    state.join("chat-attachments")
}

fn check(path: &Path, name: String, pasted_text: bool) -> ChatAttachResult {
    let Ok(meta) = std::fs::metadata(path) else {
        return refused(&name, None, "not found");
    };
    if !meta.is_file() {
        return refused(&name, None, "not a file");
    }
    let bytes = meta.len();
    if bytes > MAX_BYTES {
        return refused(&name, Some(bytes), "over 20 MB");
    }
    let ext = extension(path);
    let kind = if pasted_text {
        ChatAttachmentKind::Text
    } else if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        ChatAttachmentKind::Image
    } else if DOCUMENT_EXTENSIONS.contains(&ext.as_str()) {
        ChatAttachmentKind::File
    } else {
        match looks_like_text(path) {
            Some(true) => ChatAttachmentKind::File,
            Some(false) => return refused(&name, Some(bytes), "can't read this kind of file"),
            None => return refused(&name, Some(bytes), "couldn't read the file"),
        }
    };
    let lines = (kind == ChatAttachmentKind::Text)
        .then(|| count_lines(path))
        .flatten();
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    ChatAttachResult::Ready {
        attachment: ChatAttachment {
            name,
            path: path.to_string_lossy().into_owned(),
            kind,
            bytes,
            lines,
        },
    }
}

fn refused(name: &str, bytes: Option<u64>, reason: &str) -> ChatAttachResult {
    ChatAttachResult::Refused {
        name: name.to_owned(),
        bytes,
        reason: reason.to_owned(),
    }
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Text has no NUL byte in its first few kilobytes; `None` when the file can't be read.
fn looks_like_text(path: &Path) -> Option<bool> {
    let file = std::fs::File::open(path).ok()?;
    let mut head = Vec::new();
    file.take(SNIFF_BYTES).read_to_end(&mut head).ok()?;
    Some(!head.contains(&0))
}

fn count_lines(path: &Path) -> Option<u64> {
    let text = std::fs::read_to_string(path).ok()?;
    let lines = text.lines().count();
    u64::try_from(lines.max(1)).ok()
}

/// A file name that can't leave the staging folder: the last path part, without leading
/// dots, at most 120 characters; "file" when nothing is left.
fn safe_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let trimmed: String = last
        .trim_start_matches('.')
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    if trimmed.trim().is_empty() {
        String::from("file")
    } else {
        trimmed
    }
}

/// Writes `bytes` to a fresh file named after `name` in `dir` (private to this user), after
/// clearing staged files older than a week.
fn write_staged(dir: &Path, name: &str, bytes: &[u8], now: SystemTime) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    restrict(dir);
    prune(dir, now);
    let stamp = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    // A folder per paste keeps the file's own name, which is what the model and the chip see.
    let folder = (0u32..100)
        .map(|n| dir.join(format!("{stamp}-{n}")))
        .find(|p| std::fs::create_dir(p).is_ok())?;
    let path = folder.join(name);
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

#[cfg(unix)]
fn restrict(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict(_dir: &Path) {}

fn prune(dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age > STAGE_KEEP);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Standard base64 (with or without padding); `None` on anything else.
fn decode_base64(data: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c.checked_sub(b'A')?)),
            b'a'..=b'z' => Some(u32::from(c.checked_sub(b'a')?).checked_add(26)?),
            b'0'..=b'9' => Some(u32::from(c.checked_sub(b'0')?).checked_add(52)?),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let clean: Vec<u8> = data.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let body = clean
        .strip_suffix(b"==")
        .or_else(|| clean.strip_suffix(b"="))
        .unwrap_or(&clean);
    let mut out = Vec::with_capacity(body.len().saturating_mul(3) / 4);
    for chunk in body.chunks(4) {
        let mut acc: u32 = 0;
        for &c in chunk {
            acc = (acc << 6) | value(c)?;
        }
        let bytes = acc.to_be_bytes();
        match chunk.len() {
            4 => out.extend_from_slice(bytes.get(1..4)?),
            3 => {
                let v = (acc << 6).to_be_bytes();
                out.extend_from_slice(v.get(1..3)?);
            }
            2 => {
                let v = (acc << 12).to_be_bytes();
                out.extend_from_slice(v.get(1..2)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(result: ChatAttachResult) -> ChatAttachment {
        match result {
            ChatAttachResult::Ready { attachment } => attachment,
            ChatAttachResult::Refused { reason, .. } => panic!("refused: {reason}"),
        }
    }

    fn reason(result: ChatAttachResult) -> String {
        match result {
            ChatAttachResult::Ready { .. } => panic!("expected a refusal"),
            ChatAttachResult::Refused { reason, .. } => reason,
        }
    }

    #[test]
    fn base64_round_trips_every_padding() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("aGVsbA==").unwrap(), b"hell");
        assert_eq!(decode_base64("aGVs").unwrap(), b"hel");
        assert_eq!(decode_base64("aGVsbG8").unwrap(), b"hello");
        assert_eq!(decode_base64("").unwrap(), b"");
        assert!(decode_base64("a").is_none());
        assert!(decode_base64("a$==").is_none());
    }

    #[test]
    fn kinds_come_from_extension_and_content() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("shot.PNG");
        std::fs::write(&png, [0x89, b'P', b'N', b'G', 0, 0]).unwrap();
        let css = dir.path().join("tokens.css");
        std::fs::write(&css, "a { color: red }\n").unwrap();
        let pdf = dir.path().join("spec.pdf");
        std::fs::write(&pdf, [b'%', b'P', b'D', b'F', 0]).unwrap();
        let bin = dir.path().join("clip.mov");
        std::fs::write(&bin, [0, 1, 2, 3]).unwrap();

        assert_eq!(ready(check_path(&png)).kind, ChatAttachmentKind::Image);
        let file = ready(check_path(&css));
        assert_eq!(file.kind, ChatAttachmentKind::File);
        assert_eq!(file.name, "tokens.css");
        assert_eq!(file.lines, None);
        assert_eq!(ready(check_path(&pdf)).kind, ChatAttachmentKind::File);
        assert_eq!(reason(check_path(&bin)), "can't read this kind of file");
        assert_eq!(reason(check_path(dir.path())), "not a file");
        assert_eq!(
            reason(check_path(&dir.path().join("gone.txt"))),
            "not found"
        );
    }

    #[test]
    fn large_files_are_refused_with_their_size() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.log");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(MAX_BYTES + 1).unwrap();
        match check_path(&big) {
            ChatAttachResult::Refused { bytes, reason, .. } => {
                assert_eq!(bytes, Some(MAX_BYTES + 1));
                assert_eq!(reason, "over 20 MB");
            }
            ChatAttachResult::Ready { .. } => panic!("a 20 MB + 1 file was taken"),
        }
    }

    #[test]
    fn staged_data_keeps_its_name_and_cannot_escape() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let a = ready(stage_data(
            dir.path(),
            "../../evil/shot.png",
            "iVBORw0K",
            now,
        ));
        assert_eq!(a.name, "shot.png");
        assert_eq!(a.kind, ChatAttachmentKind::Image);
        let root = std::fs::canonicalize(dir.path()).unwrap();
        assert!(Path::new(&a.path).starts_with(&root));
        // The same name twice gets two files.
        let b = ready(stage_data(dir.path(), "shot.png", "iVBORw0K", now));
        assert_ne!(a.path, b.path);
        assert_eq!(
            reason(stage_data(dir.path(), "x.png", "%%%", now)),
            "couldn't read the file"
        );
        assert_eq!(safe_name(".."), "file");
        assert_eq!(safe_name(".env"), "env");
    }

    #[test]
    fn pasted_text_counts_lines() {
        let dir = tempfile::tempdir().unwrap();
        let t = ready(stage_text(
            dir.path(),
            "one\ntwo\nthree\n",
            SystemTime::now(),
        ));
        assert_eq!(t.kind, ChatAttachmentKind::Text);
        assert_eq!(t.name, "Pasted text");
        assert_eq!(t.lines, Some(3));
    }

    #[test]
    fn old_staged_files_are_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let then = SystemTime::now();
        let a = ready(stage_text(dir.path(), "old", then));
        let later = then + STAGE_KEEP + Duration::from_mins(1);
        let _ = ready(stage_text(dir.path(), "new", later));
        assert!(!Path::new(&a.path).exists());
    }

    #[test]
    fn compose_marks_images_and_lists_files() {
        let image = ChatAttachment {
            name: "shot.png".into(),
            path: "/tmp/a/shot.png".into(),
            kind: ChatAttachmentKind::Image,
            bytes: 10,
            lines: None,
        };
        let file = ChatAttachment {
            name: "tokens.css".into(),
            path: "/tmp/b/tokens.css".into(),
            kind: ChatAttachmentKind::File,
            bytes: 10,
            lines: None,
        };
        let text = ChatAttachment {
            name: "Pasted text".into(),
            path: "/tmp/c/pasted-text-1.txt".into(),
            kind: ChatAttachmentKind::Text,
            bytes: 10,
            lines: Some(2),
        };
        let (prompt, images) = compose("  Match this  ", &[image, file, text]);
        assert_eq!(images, vec![PathBuf::from("/tmp/a/shot.png")]);
        assert_eq!(
            prompt,
            "Match this\n\n[image: /tmp/a/shot.png]\n\nAttached files (read them at these paths):\n- /tmp/b/tokens.css\n- /tmp/c/pasted-text-1.txt (pasted text)"
        );
        let (only, _) = compose(" ", &[]);
        assert_eq!(only, "");
    }

    #[test]
    fn a_message_of_only_attachments_still_asks_something() {
        let file = ChatAttachment {
            name: "a.txt".into(),
            path: "/tmp/a.txt".into(),
            kind: ChatAttachmentKind::File,
            bytes: 1,
            lines: None,
        };
        let (prompt, _) = compose("", &[file]);
        assert!(prompt.starts_with(ONLY_ATTACHMENTS));
    }
}
