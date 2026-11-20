//! What a new file would show in `git diff`, rendered from its bytes. Pure: reading the
//! file is [`crate::changes::infrastructure::git_working_tree`]'s job.

/// Files bigger than this are not read for line counts or shown in a diff.
pub(in crate::changes) const MAX_SHOWN_BYTES: u64 = 1 << 20;

pub(in crate::changes) fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|b| *b == 0)
}

pub(in crate::changes) fn count_lines(bytes: &[u8]) -> u32 {
    let newlines = bytes.iter().filter(|b| **b == b'\n').count();
    (newlines + usize::from(bytes.last().is_some_and(|b| *b != b'\n'))) as u32
}

/// What a new file would show in `git diff`: header and one hunk of added lines. Pure.
pub(in crate::changes) fn render_new_file(path: &str, bytes: &[u8], mode: &str) -> String {
    let mut out = format!("diff --git a/{path} b/{path}\nnew file mode {mode}\n");
    if looks_binary(bytes) {
        out.push_str(&format!("Binary files /dev/null and b/{path} differ\n"));
        return out;
    }
    let text = String::from_utf8_lossy(bytes);
    let n = count_lines(bytes);
    if n == 0 {
        return out;
    }
    out.push_str(&format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +1{} @@\n", if n == 1 { String::new() } else { format!(",{n}") }));
    for line in text.split_inclusive('\n') {
        out.push('+');
        out.push_str(line.trim_end_matches('\n'));
        out.push('\n');
    }
    if !bytes.ends_with(b"\n") {
        out.push_str("\\ No newline at end of file\n");
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_renders_like_git_diff_would() {
        assert_eq!(
            render_new_file("a b.txt", b"x\ny\n", "100644"),
            "diff --git a/a b.txt b/a b.txt\nnew file mode 100644\n--- /dev/null\n+++ b/a b.txt\n@@ -0,0 +1,2 @@\n+x\n+y\n"
        );
        assert!(render_new_file("f", b"one", "100755").ends_with("@@ -0,0 +1 @@\n+one\n\\ No newline at end of file\n"));
        assert!(render_new_file("f", b"\0\x01", "100644").contains("Binary files /dev/null and b/f differ"));
        assert!(!render_new_file("empty", b"", "100644").contains("@@"));
        assert_eq!(count_lines(b"a\nb"), 2);
        assert_eq!(count_lines(b""), 0);
    }
}
