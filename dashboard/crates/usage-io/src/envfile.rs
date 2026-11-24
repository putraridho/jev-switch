//! The only reader of `~/.jev-router.env` (DESIGN 8.3, 8.5): the file goes through one
//! zeroizing buffer, and one named value comes out.

use crate::engine_bridge;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zeroize::Zeroizing;

/// The most bytes of the env file that are read; a larger file is not an env file.
const MAX_BYTES: u64 = 1024 * 1024;

/// Reads the file into a zeroizing buffer, returns `engine_bridge::parse_env(text, name)`,
/// zeroizes the buffer. `None` for a missing or unreadable file, a file that is not UTF-8 or
/// is over 1 MiB, and a missing or empty value.
pub fn read_value(path: &Path, name: &str) -> Option<Zeroizing<String>> {
    let file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size > MAX_BYTES {
        return None;
    }
    // Sized once, so reading does not reallocate and leave an unzeroized copy behind.
    let capacity = usize::try_from(size).ok()?.saturating_add(1);
    let mut buffer = Zeroizing::new(Vec::with_capacity(capacity));
    file.take(MAX_BYTES).read_to_end(&mut buffer).ok()?;
    let text = std::str::from_utf8(&buffer).ok()?;
    engine_bridge::parse_env(text, name).map(Zeroizing::new)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;

    fn file_with(text: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    fn value(text: &[u8], name: &str) -> Option<String> {
        let (_dir, path) = file_with(text);
        read_value(&path, name).map(|v| v.as_str().to_owned())
    }

    #[test]
    fn plain_export_and_quoted_values() {
        let text =
            b"A=1\nexport B = \"two\"\n  C='three'  \nexport DEEPSEEK_API_KEY=JEVCANARY_KEY_x\n";
        assert_eq!(value(text, "A").as_deref(), Some("1"));
        assert_eq!(value(text, "B").as_deref(), Some("two"));
        assert_eq!(value(text, "C").as_deref(), Some("three"));
        assert_eq!(
            value(text, "DEEPSEEK_API_KEY").as_deref(),
            Some("JEVCANARY_KEY_x")
        );
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let text = b"# A=commented\n\n   \n#export A=also\nA=real # not a comment marker here\n";
        assert_eq!(
            value(text, "A").as_deref(),
            Some("real # not a comment marker here")
        );
        assert_eq!(value(b"# A=1\n", "A"), None);
    }

    #[test]
    fn missing_key_and_empty_value_are_none() {
        assert_eq!(value(b"A=1\n", "B"), None);
        assert_eq!(value(b"A=\nB=\"\"\n", "A"), None);
        assert_eq!(value(b"A=\nB=\"\"\n", "B"), None);
        assert_eq!(value(b"MY_A=1\nA_B=2\n", "A"), None);
        assert_eq!(value(b"", "A"), None);
        assert_eq!(value(b"A=\nA=2\n", "A").as_deref(), Some("2"));
    }

    #[test]
    fn missing_file_and_directory_are_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_value(&dir.path().join("absent"), "A").is_none());
        assert!(read_value(dir.path(), "A").is_none());
    }

    #[test]
    fn non_utf8_and_oversized_files_are_none() {
        assert_eq!(value(b"A=\xff\xfe\n", "A"), None);
        let mut big = b"A=1\n".to_vec();
        big.resize(1024 * 1024 + 1, b'#');
        assert_eq!(value(&big, "A"), None);
        let mut at_limit = b"A=1\n".to_vec();
        at_limit.resize(1024 * 1024, b'#');
        assert_eq!(value(&at_limit, "A").as_deref(), Some("1"));
    }
}
