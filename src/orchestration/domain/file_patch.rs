//! A worker's patch as text: which files it touches, how much of each changed, and how to
//! page it so the planner only merges what it saw. Pure string work.

/// Most characters of diff handed back at once, in the delegate result and per `show_diff` page.
pub const MAX_DIFF_CHARS: usize = 20_000;
/// Most characters of the worker's final text handed back.
pub const MAX_SUMMARY_CHARS: usize = 4_000;

/// The old and new path in the rest of a `diff --git a/<old> b/<new>` line (after `a/`).
/// Without a rename both halves are the same text, which is checked first so a path that
/// itself contains ` b/` is still read right.
fn header_paths(rest: &str) -> (&str, &str) {
    let n = rest.len();
    if n >= 3 && (n - 3).is_multiple_of(2) {
        let half = (n - 3) / 2;
        if rest.is_char_boundary(half) && rest.is_char_boundary(half + 3) && &rest[half..half + 3] == " b/" && rest[..half] == rest[half + 3..] {
            return (&rest[..half], &rest[..half]);
        }
    }
    rest.split_once(" b/").unwrap_or((rest, rest))
}

/// The paths a unified diff touches (both sides of a rename), in order, once each.
pub fn patch_paths(patch: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in patch.lines() {
        let Some(rest) = line.strip_prefix("diff --git a/") else { continue };
        let (old, new) = header_paths(rest);
        for p in [old, new] {
            if !out.iter().any(|o| o == p) {
                out.push(p.to_string());
            }
        }
    }
    out
}

/// One file's part of a patch, exactly as git wrote it (header, hunks, binary data).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// The path after the change (the new name of a rename). A header git quoted, which
    /// this cannot parse, keeps its raw text: the file stays listed and cannot slip through.
    pub path: String,
    pub text: String,
}

/// A patch cut into per-file parts, in order. Joining the parts gives the patch back.
pub fn split_patch(patch: &str) -> Vec<FileDiff> {
    let mut out: Vec<FileDiff> = Vec::new();
    for line in patch.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let rest = rest.trim_end();
            let path = rest.strip_prefix("a/").map_or(rest, |r| header_paths(r).1);
            out.push(FileDiff { path: path.to_string(), text: String::new() });
        }
        if let Some(cur) = out.last_mut() {
            cur.text.push_str(line);
        }
    }
    out
}

pub fn join_diffs<'a>(parts: impl IntoIterator<Item = &'a FileDiff>) -> String {
    parts.into_iter().map(|p| p.text.as_str()).collect()
}

/// How much of one file changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub binary: bool,
}

impl FileStat {
    pub fn render(&self) -> String {
        if self.binary {
            format!("{} (binary)", self.path)
        } else {
            format!("{} +{} -{}", self.path, self.added, self.removed)
        }
    }
}

impl FileDiff {
    pub fn stat(&self) -> FileStat {
        let (mut added, mut removed, mut binary, mut in_hunk) = (0, 0, false, false);
        for line in self.text.lines() {
            if line.starts_with("@@") {
                in_hunk = true;
            } else if line.starts_with("GIT binary patch") || line.starts_with("Binary files ") {
                binary = true;
            } else if in_hunk {
                match line.as_bytes().first() {
                    Some(b'+') => added += 1,
                    Some(b'-') => removed += 1,
                    _ => {}
                }
            }
        }
        FileStat { path: self.path.clone(), added, removed, binary }
    }

    pub fn chars(&self) -> usize {
        self.text.chars().count()
    }
}

/// The whole files that fit in `budget` characters, in order (one that does not fit is
/// skipped, a smaller later one may still go in), as text and as the paths shown.
pub fn first_page(parts: &[FileDiff], budget: usize) -> (String, Vec<String>) {
    let (mut left, mut text, mut shown) = (budget, String::new(), Vec::new());
    for p in parts {
        let n = p.chars();
        if n <= left {
            left -= n;
            text.push_str(&p.text);
            shown.push(p.path.clone());
        }
    }
    (text, shown)
}

/// `max` characters of `text` from character `offset`, and the offset just after them.
pub fn char_page(text: &str, offset: usize, max: usize) -> (&str, usize) {
    let byte = |chars: usize| text.char_indices().nth(chars).map_or(text.len(), |(b, _)| b);
    let (from, to) = (byte(offset), byte(offset.saturating_add(max)));
    (&text[from..to], offset + text[from..to].chars().count())
}

/// `text` cut to `max` characters, with a note of how much was dropped.
pub fn truncate_with_note(text: &str, max: usize, what: &str) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}\n[{what} truncated: {} more characters not shown]", total - max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::unified_patch::{big_patch, file_patch};

    #[test]
    fn patch_paths_lists_each_touched_file_once() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-x\n+y\n\
                     diff --git a/old.txt b/new.txt\nrename from old.txt\nrename to new.txt\n\
                     diff --git a/src/a.rs b/src/a.rs\n";
        assert_eq!(patch_paths(patch), ["src/a.rs", "old.txt", "new.txt"]);
        assert!(patch_paths("").is_empty());
    }

    #[test]
    fn long_text_is_cut_with_a_note() {
        assert_eq!(truncate_with_note("short", 10, "diff"), "short");
        let cut = truncate_with_note(&"x".repeat(30), 10, "diff");
        assert!(cut.starts_with(&"x".repeat(10)) && cut.contains("diff truncated: 20 more characters"), "{cut}");
    }

    #[test]
    fn a_patch_splits_per_file_and_counts_changes() {
        let patch = format!(
            "{}{}diff --git a/img.png b/img.png\nnew file mode 100644\nindex 0000000..1111111\nGIT binary patch\nliteral 3\nKcmZ?)\n\n",
            file_patch("src/a.rs", "old", &["n1", "n2"]),
            "diff --git a/old.txt b/new.txt\nsimilarity index 90%\nrename from old.txt\nrename to new.txt\n--- a/old.txt\n+++ b/new.txt\n@@ -1 +1 @@\n-x\n+y\n"
        );
        let parts = split_patch(&patch);
        assert_eq!(parts.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), ["src/a.rs", "new.txt", "img.png"]);
        assert_eq!(join_diffs(&parts), patch, "the parts add up to the patch");
        let stats: Vec<_> = parts.iter().map(FileDiff::stat).collect();
        assert_eq!((stats[0].added, stats[0].removed), (2, 1));
        assert_eq!((stats[1].added, stats[1].removed), (1, 1), "the +++ and --- header lines are not counted");
        assert!(stats[2].binary && stats[2].render() == "img.png (binary)");
        assert!(split_patch("").is_empty());
        // A path that itself contains " b/" is read whole; a quoted header keeps its raw text, so it is still listed.
        assert_eq!(split_patch("diff --git a/x b/y b/x b/y\n")[0].path, "x b/y");
        assert_eq!(split_patch("diff --git \"a/caf\\303\\251\" \"b/caf\\303\\251\"\n")[0].path, "\"a/caf\\303\\251\" \"b/caf\\303\\251\"");
    }

    #[test]
    fn the_first_page_holds_whole_files_only() {
        let parts = split_patch(&format!("{}{}{}", big_patch("big.rs", 5000), file_patch("small.rs", "a", &["b"]), big_patch("big2.rs", 100)));
        let (text, shown) = first_page(&parts, 3000);
        assert_eq!(shown, ["small.rs", "big2.rs"], "a file that does not fit is skipped, a later small one still goes in");
        assert!(text.contains("small.rs") && !text.contains("big.rs"));
        assert_eq!(char_page("héllo wörld", 3, 4), ("lo w", 7), "offsets count characters");
        assert_eq!(char_page("abc", 5, 4), ("", 5));
    }
}
