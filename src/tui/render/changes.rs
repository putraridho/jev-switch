//! What the agent changed: the per-file list, coloured diffs and tool-call lines.

use std::path::Path;

use crate::tui::render::style::Style;
use crate::tui::render::text::one_line;
use crate::tui::render::width::{clamp_width, display_width, pad_to_width, truncate_start_to_width, truncate_to_width};

/// One changed file, from `git status`/`git diff --numstat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// `A`, `M`, `D`, `R` (or `?` for untracked, shown like `A`).
    pub status: char,
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// Most files [`changes_block`] lists before saying "… and N more".
pub const MAX_CHANGE_ROWS: usize = 8;

fn status_letter(c: char, style: Style) -> String {
    let s = c.to_string();
    match c {
        'A' | '?' => style.green(&s),
        'M' => style.yellow(&s),
        'D' => style.red(&s),
        'R' => style.cyan(&s),
        _ => style.dim(&s),
    }
}

/// The compact per-file list under the summary: coloured status letter, path (cut from
/// the left to fit), `+n −n`. At most [`MAX_CHANGE_ROWS`] files, then `… and N more`.
/// Empty string for no files. No line is wider than `width`.
pub fn changes_block(files: &[FileChange], style: Style, width: usize) -> String {
    if files.is_empty() {
        return String::new();
    }
    let width = clamp_width(width);
    let shown = &files[..files.len().min(MAX_CHANGE_ROWS)];
    let counts: Vec<(String, String)> = shown
        .iter()
        .map(|f| if f.added == 0 && f.removed == 0 { (String::new(), String::new()) } else { (format!("+{}", f.added), format!("−{}", f.removed)) })
        .collect();
    let plus_w = counts.iter().map(|(a, _)| display_width(a)).max().unwrap_or(0);
    let minus_w = counts.iter().map(|(_, r)| display_width(r)).max().unwrap_or(0);
    let counts_w = if plus_w == 0 { 0 } else { plus_w + 1 + minus_w };
    let fixed = 4 + if counts_w > 0 { 2 + counts_w } else { 0 };
    let budget = width.saturating_sub(fixed).max(1);
    let paths: Vec<String> = shown.iter().map(|f| truncate_start_to_width(&one_line(&f.path), budget)).collect();
    let path_w = paths.iter().map(|p| display_width(p)).max().unwrap_or(0);
    let mut lines: Vec<String> = Vec::new();
    for ((f, path), (plus, minus)) in shown.iter().zip(&paths).zip(&counts) {
        let mut line = format!("  {} {}", status_letter(f.status, style), pad_to_width(path, path_w));
        if counts_w > 0 {
            if plus.is_empty() {
                line.push_str(&" ".repeat(2 + counts_w));
            } else {
                let pad = " ".repeat(plus_w - display_width(plus));
                line.push_str(&format!("  {pad}{} {}", style.green(plus), style.red(minus)));
            }
        }
        lines.push(truncate_to_width(line.trim_end(), width));
    }
    if files.len() > shown.len() {
        lines.push(style.dim(&format!("  … and {} more", files.len() - shown.len())));
    }
    lines.join("\n")
}

/// A unified diff coloured: file headers bold, `+` lines green, `-` lines red, `@@`
/// hunks cyan. Line structure (including a trailing newline) is preserved; a disabled
/// style returns the text unchanged.
pub fn diff_colorize(text: &str, style: Style) -> String {
    if !style.enabled {
        return text.to_string();
    }
    const HEADERS: [&str; 8] = ["diff ", "index ", "--- ", "+++ ", "new file", "deleted file", "similarity ", "rename "];
    text.split('\n')
        .map(|line| {
            let body = line.strip_suffix('\r').unwrap_or(line);
            if HEADERS.iter().any(|h| body.starts_with(h)) {
                style.bold(line)
            } else if body.starts_with("@@") {
                style.cyan(line)
            } else if body.starts_with('+') {
                style.green(line)
            } else if body.starts_with('-') {
                style.red(line)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// --------------------------------------------------------------- tool lines

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || "/._-~+@%".contains(c)
}

/// `text` with absolute paths inside `project_dir` made relative to it (`/p/src/a.rs` →
/// `src/a.rs`, `/p` → `.`). A sibling directory that merely starts with the same name
/// (`/p2/x`) is left alone.
pub fn relativize_paths(text: &str, project_dir: &Path) -> String {
    let dir = project_dir.to_string_lossy();
    let dir = dir.trim_end_matches('/');
    if dir.is_empty() {
        return text.to_string();
    }
    let (mut out, mut last) = (String::new(), 0);
    for (i, _) in text.match_indices(dir) {
        if i < last || text[..i].chars().next_back().is_some_and(is_path_char) {
            continue;
        }
        let after = &text[i + dir.len()..];
        match after.chars().next() {
            Some('/') => {
                out.push_str(&text[last..i]);
                if after[1..].chars().next().is_none_or(|c| !is_path_char(c)) {
                    out.push('.');
                }
                last = i + dir.len() + 1;
            }
            Some(c) if is_path_char(c) => {}
            _ => {
                out.push_str(&text[last..i]);
                out.push('.');
                last = i + dir.len();
            }
        }
    }
    out.push_str(&text[last..]);
    out
}

/// One tool call, dim: `  ⏺ Edit src/a.rs`. Paths inside the project are relative,
/// whitespace and newlines are collapsed to single spaces, and the line is cut with an
/// ellipsis to `width`.
pub fn tool_line(summary: &str, project_dir: &Path, style: Style, width: usize) -> String {
    let body = relativize_paths(&one_line(summary), project_dir);
    let (verb, detail) = body.split_once(' ').unwrap_or((&body, ""));
    let line = if detail.is_empty() {
        format!("  {} {}", style.cyan("⏺"), style.bold(verb))
    } else {
        format!("  {} {} {}", style.cyan("⏺"), style.bold(verb), style.dim(detail))
    };
    truncate_to_width(&line, clamp_width(width))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::render::{strip_ansi, MIN_WIDTH};
    use crate::tui::render::test_facts::*;

    #[test]
    fn changes_block_lists_and_aligns() {
        let files = vec![fc('M', "src/a.rs", 12, 4), fc('A', "src/very_long_name.rs", 3, 0), fc('D', "old.txt", 0, 20)];
        assert_eq!(
            changes_block(&files, OFF, 80),
            "  M src/a.rs               +12 −4\n  A src/very_long_name.rs   +3 −0\n  D old.txt                 +0 −20"
        );
        assert_eq!(changes_block(&[], OFF, 80), "");
        // binary: no counts, no trailing spaces
        assert_eq!(changes_block(&[fc('M', "logo.png", 0, 0)], OFF, 80), "  M logo.png");
    }

    #[test]
    fn changes_block_caps_at_eight() {
        let files: Vec<FileChange> = (0..11).map(|i| fc('M', &format!("f{i}.rs"), 1, 1)).collect();
        let s = changes_block(&files, OFF, 60);
        let ls = lines(&s);
        assert_eq!(ls.len(), MAX_CHANGE_ROWS + 1);
        assert_eq!(ls[MAX_CHANGE_ROWS], "  … and 3 more");
        assert!(!s.contains("f8.rs"));
        let exact: Vec<FileChange> = files.into_iter().take(8).collect();
        assert!(!changes_block(&exact, OFF, 60).contains("more"));
    }

    #[test]
    fn changes_block_respects_width() {
        let files = vec![
            fc('M', "src/some/really/deep/directory/structure/with/a/file_name.rs", 120, 400),
            fc('A', "日本語/ファイル.rs", 1, 0),
            fc('M', "a\x1b[31m.rs", 1, 1),
        ];
        for w in [MIN_WIDTH, 24, 30, 40, 60, 100] {
            for style in [ON, OFF] {
                for l in changes_block(&files, style, w).split('\n') {
                    assert!(display_width(l) <= w, "{l:?} at {w}");
                }
            }
        }
        let s = changes_block(&files[..1], OFF, 40);
        assert!(s.contains("…") && s.contains("file_name.rs"), "{s}");
        no_esc(&changes_block(&files, OFF, 40));
    }

    #[test]
    fn changes_block_colours_statuses() {
        let files = vec![fc('A', "a", 1, 0), fc('M', "m", 1, 0), fc('D', "d", 0, 1), fc('?', "u", 0, 0), fc('R', "r", 0, 0)];
        let s = changes_block(&files, ON, 60);
        assert!(s.contains("\x1b[32mA\x1b[0m"));
        assert!(s.contains("\x1b[33mM\x1b[0m"));
        assert!(s.contains("\x1b[31mD\x1b[0m"));
        assert!(s.contains("\x1b[32m?\x1b[0m"));
        assert!(s.contains("\x1b[36mR\x1b[0m"));
        assert!(s.contains("\x1b[32m+1\x1b[0m") && s.contains("\x1b[31m−0\x1b[0m"));
        assert_eq!(strip_ansi(&s), changes_block(&files, OFF, 60));
    }

    #[test]
    fn diff_colours() {
        let diff = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n context\n-old\n+new\n";
        let s = diff_colorize(diff, ON);
        let ls = lines(&s);
        assert_eq!(ls[0], "\x1b[1mdiff --git a/x b/x\x1b[0m");
        assert_eq!(ls[2], "\x1b[1m--- a/x\x1b[0m");
        assert_eq!(ls[3], "\x1b[1m+++ b/x\x1b[0m");
        assert_eq!(ls[4], "\x1b[36m@@ -1,2 +1,2 @@\x1b[0m");
        assert_eq!(ls[5], " context");
        assert_eq!(ls[6], "\x1b[31m-old\x1b[0m");
        assert_eq!(ls[7], "\x1b[32m+new\x1b[0m");
        assert!(s.ends_with('\n'), "a trailing newline survives");
        assert_eq!(strip_ansi(&s), diff);
        assert_eq!(diff_colorize(diff, OFF), diff);
        assert_eq!(diff_colorize("", ON), "");
        assert_eq!(diff_colorize("+a\r\n", ON), "\x1b[32m+a\r\x1b[0m\n");
    }

    #[test]
    fn tool_lines() {
        let dir = Path::new("/home/me/proj");
        for (summary, want) in [
            ("Edit /home/me/proj/src/a.rs", "  ⏺ Edit src/a.rs"),
            ("Edit src/a.rs", "  ⏺ Edit src/a.rs"),
            ("Read /home/me/proj2/x.rs", "  ⏺ Read /home/me/proj2/x.rs"),
            ("Read /home/me/proj-old/x.rs", "  ⏺ Read /home/me/proj-old/x.rs"),
            ("Bash cd /home/me/proj && cargo test", "  ⏺ Bash cd . && cargo test"),
            ("Bash ls /home/me/proj/", "  ⏺ Bash ls ."),
            ("Grep foo --path=/home/me/proj/src", "  ⏺ Grep foo --path=src"),
            ("Edit /elsewhere/a.rs", "  ⏺ Edit /elsewhere/a.rs"),
            ("Bash echo a\n  b\tc", "  ⏺ Bash echo a b c"),
            ("Write /home/me/proj/a.rs and /home/me/proj/b.rs", "  ⏺ Write a.rs and b.rs"),
        ] {
            assert_eq!(tool_line(summary, dir, OFF, 80), want, "{summary}");
        }
        // the project dir with a trailing slash behaves the same; the root dir changes nothing
        assert_eq!(tool_line("Edit /p/a.rs", Path::new("/p/"), OFF, 80), "  ⏺ Edit a.rs");
        assert_eq!(tool_line("Edit /p/a.rs", Path::new("/"), OFF, 80), "  ⏺ Edit /p/a.rs");
    }

    #[test]
    fn tool_lines_are_cut_to_width() {
        let dir = Path::new("/p");
        let cmd = format!("Bash {}", "cargo test --all-targets ".repeat(10));
        for w in [MIN_WIDTH, 30, 50, 120] {
            let s = tool_line(&cmd, dir, ON, w);
            assert!(display_width(&s) <= w, "{w}");
            assert!(strip_ansi(&s).ends_with('…') || w >= 120);
        }
        assert_eq!(tool_line("Bash 日本語日本語日本語日本語日本語日本語", dir, OFF, 20), "  ⏺ Bash 日本語日本…");
        let hostile = tool_line("Bash \x1b[2Jrm", dir, OFF, 40);
        no_esc(&hostile);
        assert_eq!(hostile, "  ⏺ Bash rm");
    }

    #[test]
    fn tool_line_separates_action_from_detail() {
        let line = tool_line("Edit a", Path::new("/p"), ON, 40);
        assert!(line.contains("\x1b[1mEdit\x1b[0m"));
        assert!(line.contains("\x1b[2ma\x1b[0m"));
    }
}
