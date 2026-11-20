//! Paths pasted into a prompt, found the way a terminal pastes a dropped file: quoted
//! (`'/a/b c.png'`), with escaped spaces (`/a/b\ c.png`), as a `file://` URL, from `~/`,
//! or mentioned with `@path`. Only paths that exist count. Images are taken out of the
//! text (the backend sends them as images); files and folders are written as a plain
//! path the model can open. Text without paths comes back byte for byte.

use std::path::{Path, PathBuf};

/// A prompt with its pasted paths found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Attached {
    /// The prompt, with each image marked `[image: path]` and each file or folder
    /// written as a plain path.
    pub text: String,
    /// Absolute paths of the images.
    pub images: Vec<PathBuf>,
    /// Files, relative to the project when inside it.
    pub files: Vec<PathBuf>,
    /// Folders, relative to the project when inside it.
    pub dirs: Vec<PathBuf>,
}

/// Most images sent with one message; more are left as paths.
pub const MAX_IMAGES: usize = 10;

/// Extensions sent as images.
const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// One whitespace-separated word of the prompt, with quotes and escapes resolved.
struct Token<'a> {
    /// The word as typed.
    original: &'a str,
    /// The word with quotes removed and escapes applied.
    value: String,
    /// Typed the way a terminal pastes a dropped path (quoted or with escaped spaces).
    pasted: bool,
    /// Has a backslash-escaped space.
    escaped: bool,
}

/// What a token turned out to be on disk.
enum Found {
    Image(PathBuf),
    File(PathBuf),
    Dir(PathBuf),
}

/// Find the pasted paths in `prompt`; relative ones are looked up in `project`.
pub fn extract(prompt: &str, project: &Path) -> Attached {
    let mut result = Attached::default();
    let mut rest = prompt;
    while !rest.is_empty() {
        let blank = rest.len() - rest.trim_start().len();
        result.text.push_str(&rest[..blank]);
        rest = &rest[blank..];
        if rest.is_empty() {
            break;
        }
        let token = next_token(rest);
        rest = &rest[token.original.len()..];
        match lookup(&token, project) {
            Some(Found::Image(path)) if result.images.len() < MAX_IMAGES => {
                // The path stays readable, so the model can also open or change the file.
                let shown = project.canonicalize().ok().and_then(|base| path.strip_prefix(base).ok().map(Path::to_path_buf)).unwrap_or_else(|| path.clone());
                result.text.push_str(&format!("[image: {}]", shown.display()));
                result.images.push(path);
            }
            Some(Found::File(path)) => {
                result.text.push_str(&shown_path(&token, &path));
                result.files.push(path);
            }
            Some(Found::Dir(path)) => {
                result.text.push_str(&shown_path(&token, &path));
                result.dirs.push(path);
            }
            Some(Found::Image(_)) | None => result.text.push_str(token.original),
        }
    }
    result
}

/// How a found file or folder reads in the text. A form the model could not open as is
/// (`@path`, `file://`, escaped spaces) becomes a plain path, `.` for the project itself;
/// anything else, quoted words included, stays as typed.
fn shown_path(token: &Token<'_>, found: &Path) -> String {
    let rewrite = token.escaped || token.original.starts_with('@') || token.original.starts_with("file://");
    if !rewrite {
        return token.original.to_string();
    }
    let text = found.to_string_lossy();
    if text.is_empty() {
        ".".to_string()
    } else {
        text.into_owned()
    }
}

/// The word at the start of `s` (which does not start with whitespace). A quote runs to
/// its closing quote, a backslash keeps the next space, quote or backslash in the word.
fn next_token(s: &str) -> Token<'_> {
    let mut value = String::new();
    let mut quote: Option<char> = None;
    let mut escaped_space = false;
    let mut chars = s.char_indices().peekable();
    let mut end = s.len();
    while let Some((i, ch)) = chars.next() {
        match (quote, ch) {
            (None, c) if c.is_whitespace() => {
                end = i;
                break;
            }
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (q, '\\') if q != Some('\'') => match chars.peek() {
                Some(&(_, next)) if next.is_whitespace() || matches!(next, '\\' | '"' | '\'') => {
                    escaped_space |= next.is_whitespace();
                    value.push(next);
                    chars.next();
                }
                _ => value.push('\\'),
            },
            _ => value.push(ch),
        }
    }
    let original = &s[..end];
    // An unclosed quote is not a path.
    let pasted = quote.is_none() && (original.starts_with(['\'', '"']) || escaped_space);
    if quote.is_some() {
        value.clear();
    }
    Token { original, value, pasted, escaped: escaped_space && quote.is_none() }
}

/// The path a token names, when it is one that exists.
fn lookup(token: &Token<'_>, project: &Path) -> Option<Found> {
    let value = token.value.as_str();
    if value.is_empty() || (value.contains("://") && !value.starts_with("file://")) {
        return None;
    }
    let mentioned = value.starts_with('@');
    let value = value.strip_prefix('@').unwrap_or(value);
    let text = if let Some(url) = value.strip_prefix("file://") {
        percent_decode(url)?
    } else if let Some(rest) = value.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME")?).join(rest).to_string_lossy().into_owned()
    } else {
        value.to_string()
    };
    // A bare word is prose ("fix main"), unless it was mentioned or pasted.
    if !mentioned && !token.pasted && !value.starts_with("file://") && !text.contains('/') {
        return None;
    }
    let path = Path::new(&text);
    let full = if path.is_absolute() { path.to_path_buf() } else { project.join(path) }.canonicalize().ok()?;
    if full.is_file() && is_image(&full) {
        return Some(Found::Image(full));
    }
    let base = project.canonicalize().ok()?;
    let shown = full.strip_prefix(&base).map(Path::to_path_buf).unwrap_or_else(|_| full.clone());
    if full.is_file() {
        Some(Found::File(shown))
    } else if full.is_dir() {
        Some(Found::Dir(shown))
    } else {
        None
    }
}

/// Whether the file is sent as an image, by its extension.
pub fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// `%20` and friends of a `file://` URL, decoded; `None` when it is not valid UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(s.len());
    let mut iter = s.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let hi = char::from(iter.next()?).to_digit(16)?;
            let lo = char::from(iter.next()?).to_digit(16)?;
            bytes.push(u8::try_from(hi * 16 + lo).ok()?);
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-attach-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("a b.PNG"), b"x").unwrap();
        std::fs::write(dir.join("shot.png"), b"x").unwrap();
        std::fs::write(dir.join("src/main.rs"), b"fn main() {}").unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn dropped_images_in_every_pasted_form_are_taken_out_of_the_text() {
        let dir = project("forms");
        let abs = dir.join("a b.PNG");
        let forms = [
            format!("'{}'", abs.display()),
            format!("\"{}\"", abs.display()),
            abs.display().to_string().replace(' ', "\\ "),
            format!("file://{}", abs.display().to_string().replace(' ', "%20")),
            "@a\\ b.PNG".to_string(),
        ];
        for form in forms {
            let got = extract(&format!("what is in {form} ?"), &dir);
            assert_eq!(got.text, "what is in [image: a b.PNG] ?", "{form}");
            assert_eq!(got.images, std::slice::from_ref(&abs), "{form}");
        }
    }

    #[test]
    fn files_and_folders_become_plain_paths_relative_to_the_project() {
        let dir = project("files");
        let src = dir.join("src").display().to_string();
        let got = extract(&format!("look at @src/main.rs and {src}"), &dir);
        assert_eq!(got.text, format!("look at src/main.rs and {src}"), "a plainly typed path stays as typed");
        assert_eq!(got.files, [PathBuf::from("src/main.rs")]);
        assert_eq!(got.dirs, [PathBuf::from("src")]);
        let got = extract("fix src/main.rs", &dir);
        assert_eq!(got.text, "fix src/main.rs");
        assert_eq!(extract("@.", &dir).text, ".", "the project itself reads as `.`");
        std::fs::write(dir.join("src/logo.png"), b"x").unwrap();
        let got = extract("convert src/logo.png to webp", &dir);
        assert_eq!(got.text, "convert [image: src/logo.png] to webp", "an image keeps its path, so the model can change the file");
        assert_eq!(got.images.len(), 1);
        let json = "{\"path\": \"src/main.rs\"}";
        assert_eq!(extract(json, &dir).text, json, "quoted words in pasted text keep their quotes");
        let escaped = format!("{}/my\\ notes.md", dir.display());
        std::fs::write(dir.join("my notes.md"), b"x").unwrap();
        assert_eq!(extract(&escaped, &dir).text, "my notes.md", "an escaped path becomes plain");
    }

    #[test]
    fn text_without_real_paths_comes_back_unchanged() {
        let dir = project("prose");
        for text in [
            "@missing",
            "https://x/y.png",
            "prose with a/b slash",
            "shot.png alone is a bare word",
            "fix the bug\n\nthanks  ",
            "an 'unclosed quote",
            "C:\\ path\\",
            "",
        ] {
            let got = extract(text, &dir);
            assert_eq!(got.text, text);
            assert!(got.images.is_empty() && got.files.is_empty() && got.dirs.is_empty(), "{text}");
        }
    }

    #[test]
    fn at_most_ten_images_go_with_one_message() {
        let dir = project("many");
        let words: Vec<String> = (0..12)
            .map(|i| {
                std::fs::write(dir.join(format!("p{i}.png")), b"x").unwrap();
                format!("@p{i}.png")
            })
            .collect();
        let got = extract(&words.join(" "), &dir);
        assert_eq!(got.images.len(), MAX_IMAGES);
        assert!(got.text.ends_with("@p10.png @p11.png"), "{}", got.text);
    }

    #[test]
    fn image_extensions_ignore_case() {
        assert!(is_image(Path::new("a.JPG")) && is_image(Path::new("b.webp")));
        assert!(!is_image(Path::new("c.svg")) && !is_image(Path::new("png")));
    }

    #[test]
    fn percent_decoding_rejects_broken_escapes() {
        assert_eq!(percent_decode("/a%20b").as_deref(), Some("/a b"));
        assert_eq!(percent_decode("/a%2"), None);
        assert_eq!(percent_decode("/a%zz"), None);
    }
}
