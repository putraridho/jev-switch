//! Unified-diff text for tests.


/// A one-file patch: `old` replaced by `new` lines.
pub fn file_patch(path: &str, old: &str, new: &[&str]) -> String {
    let added: String = new.iter().map(|l| format!("+{l}\n")).collect();
    format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1,{} @@\n-{old}\n{added}", new.len())
}

/// A one-file patch of about `chars` characters.
pub fn big_patch(path: &str, chars: usize) -> String {
    let line = "x".repeat(99);
    file_patch(path, "a", &vec![line.as_str(); chars / 100])
}

