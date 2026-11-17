//! The tool-approval question, its answer line, the "denied without asking" hint and the
//! `/permissions` screen.

use std::path::Path;

use super::changes::relativize_paths;
use super::style::Style;
use super::text::one_line;
use super::width::{char_width, clamp_width, display_width};

/// The most rows of input the question prints itself; more goes through the pager, so that
/// the whole of it is on screen before a key approves it.
const MAX_INLINE_ROWS: usize = 20;

/// Rows of a long input kept in the question next to the pager's.
const PEEK_ROWS: usize = 6;

/// What to ask. `lines` is everything the answer approves, already safe to print (see
/// `permissions::review`); it is wrapped, never cut.
pub struct Question<'a> {
    pub tool: &'a str,
    /// Facts about the call that are not its input, printed apart from the input rows.
    pub notes: &'a [String],
    pub lines: &'a [String],
    /// Whether the `a` choice is offered.
    pub always: bool,
    /// A dim line under the choices: why `a` is missing, or what it would also allow.
    pub note: Option<&'a str>,
    pub project_dir: &'a Path,
    pub style: Style,
    pub width: usize,
}

/// A laid-out question.
pub struct Prompt {
    /// What to print once everything the answer approves has been shown. When the input is
    /// long this keeps its first rows and says that the pager showed the rest.
    pub text: String,
    /// For a long input, all of it (the question's first line and every row): the caller
    /// pages it, or, with no pager to run, prints it and then [`Prompt::tail`] instead of
    /// [`Prompt::text`].
    pub full: Option<String>,
    /// The choices and the note.
    pub tail: String,
}

/// `text` as rows of at most `width` columns, cut by display width and never dropped. Every
/// character is kept, so a row break is the only change.
fn wrap_hard(text: &str, width: usize) -> Vec<String> {
    let (mut rows, mut row, mut used) = (Vec::new(), String::new(), 0);
    for c in text.chars() {
        let w = char_width(c);
        if used + w > width && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
            used = 0;
        }
        row.push(c);
        used += w;
    }
    rows.push(row);
    rows
}

/// The choices on one line: each key letter cyan (`[y]es`), the rest dim.
fn choices(always: bool, style: Style) -> String {
    let items: &[&str] = if always { &["[y]es", "[n]o", "[a]lways in this project"] } else { &["[y]es", "[n]o"] };
    items
        .iter()
        .map(|item| match item.split_once(']') {
            Some((key, rest)) => format!("{}{}", style.cyan(&format!("{key}]")), style.dim(rest)),
            None => (*item).to_string(),
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// The question for one tool call. A short single-line input and the choices share a line
/// (`Allow Bash: cargo test --quiet?  [y]es  [n]o  [a]lways in this project`); anything else
/// is the first line, every row of the input wrapped to the width (a gutter marks where a
/// line starts and where one only continues), then the choices. Nothing is cut: an input
/// of more than a screenful comes as [`Prompt::full`] too.
pub fn permission_prompt(q: &Question) -> Prompt {
    let (style, width) = (q.style, clamp_width(q.width).saturating_sub(1));
    let tool = one_line(q.tool);
    let lines: Vec<String> = q.lines.iter().map(|l| relativize_paths(l, q.project_dir)).collect();
    let choices_plain = if q.always { "[y]es  [n]o  [a]lways in this project" } else { "[y]es  [n]o" };
    let note_rows: Vec<String> = q.note.map(|n| wrap_hard(&one_line(n), width).into_iter().map(|r| style.dim(&r)).collect()).unwrap_or_default();
    let join = |parts: &[&[String]]| parts.iter().flat_map(|p| p.iter()).map(String::as_str).collect::<Vec<_>>().join("\n");

    let head = format!("Allow {tool}: ");
    if let ([line], true) = (lines.as_slice(), q.notes.is_empty()) {
        if display_width(&head) + display_width(line) + 1 + 2 + display_width(choices_plain) <= width {
            let first = format!("{}{line}{}  {}", style.bold(&head), style.bold("?"), choices(q.always, style));
            return Prompt { text: join(&[&[first], &note_rows]), full: None, tail: String::new() };
        }
    }

    let text_width = width.saturating_sub(2).max(1);
    let rows: Vec<String> = lines
        .iter()
        .flat_map(|l| {
            wrap_hard(l, text_width).into_iter().enumerate().map(|(i, r)| format!("{}{r}", style.dim(if i == 0 { "\u{2502} " } else { "\u{2506} " })))
        })
        .collect();
    let mut top = vec![style.bold(&format!("Allow {tool}:"))];
    if !q.notes.is_empty() {
        top.extend(wrap_hard(&format!("options: {}", q.notes.join(" ")), width).into_iter().map(|r| style.yellow(&r)));
    }
    let tail = join(&[&[choices(q.always, style)], &note_rows]);
    if rows.len() <= MAX_INLINE_ROWS {
        return Prompt { text: join(&[&top, &rows, &[tail.clone()]]), full: None, tail };
    }
    let more = style.dim(&format!(
        "\u{2026} {} more rows ({} lines in all): the pager above showed all of it",
        rows.len() - PEEK_ROWS,
        lines.len()
    ));
    let text = join(&[&top, &rows[..PEEK_ROWS], &[more], &[tail.clone()]]);
    Prompt { text, full: Some(join(&[&top, &rows])), tail }
}

/// What the answer to an approval question looks like once given.
pub fn permission_answered(decision: Option<&crate::backend::Decision>, rule: Option<&str>, style: Style) -> String {
    use crate::backend::Decision;
    match decision {
        Some(Decision::Allow) => style.green("\u{2713} allowed"),
        Some(Decision::AllowAlways) => match rule {
            Some(rule) => format!("{} {}", style.green("\u{2713} allowed, and always in this project:"), one_line(rule)),
            None => style.green("\u{2713} allowed"),
        },
        Some(Decision::Deny { .. }) => style.yellow("\u{2717} denied"),
        None => style.yellow("\u{25a0} stopped"),
    }
}

/// The hint when a tool call was denied because nobody could be asked (`-p`, a pipe).
pub fn denied_without_asking(tool: &str) -> String {
    format!(
        "\u{2717} denied {} \u{2014} nobody to ask in this run; allow it with CODE_ALLOWED_TOOLS, or in an interactive session answer the question or use /permissions",
        one_line(tool)
    )
}

/// The `/permissions` screen: the project's rules and folders, then the user-level rules.
pub fn permissions_text(l: &crate::permissions::Listing, style: Style, project_dir: &Path) -> String {
    let show = |path: &Path| relativize_paths(&path.display().to_string(), project_dir);
    let mut lines = vec![style.bold("Permissions"), format!("  {}", style.dim(&format!("project rules ({})", show(&l.local_path))))];
    if l.allow.is_empty() {
        lines.push(format!("    {}", style.dim("none yet: answer `a` at a question, or /permissions allow <rule>")));
    }
    lines.extend(l.allow.iter().map(|r| format!("    {}", one_line(r))));
    if !l.directories.is_empty() {
        lines.push(format!("  {}", style.dim("additional folders")));
        lines.extend(l.directories.iter().map(|d| format!("    {}", one_line(d))));
    }
    if !l.user_allow.is_empty() {
        let path = l.user_path.as_deref().map(|p| format!(" ({})", p.display())).unwrap_or_default();
        lines.push(format!("  {}", style.dim(&format!("your rules{path}, read only"))));
        lines.extend(l.user_allow.iter().map(|r| format!("    {}", one_line(r))));
    }
    lines.extend(l.problems.iter().map(|p| format!("  {} {}", style.yellow("\u{26a0}"), one_line(p))));
    lines.push(format!(
        "  {}",
        style.dim("Rules apply to Claude and DeepSeek (both run through Claude Code). Codex, Gemini and Grok decide for themselves.")
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::testutil::*;
    use crate::uifmt::{command_names, command_usage, complete, help_text, strip_ansi, CompleteCtx, MIN_WIDTH};

    // ---------------------------------------------------------- permissions

    const PROJECT: &str = "/proj";

    /// Lay out a question about `lines` with the defaults the tests share.
    fn ask(lines: &[&str], always: bool, style: Style, width: usize) -> Prompt {
        let lines: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();
        permission_prompt(&Question { tool: "Bash", notes: &[], lines: &lines, always, note: None, project_dir: Path::new(PROJECT), style, width })
    }

    fn many(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("echo line-{i}")).collect()
    }

    fn rows_of(p: &Prompt) -> Vec<&str> {
        p.text.lines().collect()
    }

    #[test]
    fn a_short_command_and_the_choices_share_one_line() {
        assert_eq!(ask(&["cargo test --quiet"], true, OFF, 100).text, "Allow Bash: cargo test --quiet?  [y]es  [n]o  [a]lways in this project");
        assert_eq!(ask(&["x"], false, OFF, 100).text, "Allow Bash: x?  [y]es  [n]o");
        // Paths in the project are shown relative, like tool lines.
        let p = permission_prompt(&Question {
            tool: "Write",
            notes: &[],
            lines: &["/proj/src/a.rs (12 bytes)".to_string()],
            always: true,
            note: None,
            project_dir: Path::new(PROJECT),
            style: OFF,
            width: 100,
        });
        assert!(p.text.contains("Allow Write: src/a.rs (12 bytes)?"), "{}", p.text);
    }

    #[test]
    fn a_command_that_does_not_fit_is_wrapped_never_cut() {
        let long = format!("echo {}", "x".repeat(300));
        // The choices alone are 38 columns, so the question needs at least 40.
        for width in [40usize, 50, 60, 80, 120] {
            let p = ask(&[&long], true, OFF, width);
            for line in p.text.lines() {
                assert!(display_width(line) < width.max(MIN_WIDTH), "width {width}: {line:?}");
            }
            assert!(p.text.contains("[y]es") && p.text.contains("[n]o") && p.text.contains("[a]lways"), "{}", p.text);
            assert!(!p.text.contains('\u{2026}'), "nothing is cut: {}", p.text);
            let kept: String = p.text.lines().filter_map(|l| l.strip_prefix("\u{2502} ").or_else(|| l.strip_prefix("\u{2506} "))).collect();
            assert_eq!(kept, long, "every character is on screen at width {width}");
            assert!(p.full.is_none(), "{width}");
        }
        assert_eq!(ask(&[&long], true, OFF, 100).text.lines().count(), 1 + 4 + 1);
    }

    #[test]
    fn every_line_of_a_multi_line_command_is_shown_with_a_gutter_and_a_continuation_mark() {
        let p = ask(&["echo hi", "rm -rf ~", &"y".repeat(60)], true, OFF, 40);
        let rows = rows_of(&p);
        assert_eq!(rows[0], "Allow Bash:");
        assert_eq!(rows[1], "\u{2502} echo hi");
        assert_eq!(rows[2], "\u{2502} rm -rf ~");
        assert!(rows[3].starts_with("\u{2502} y") && rows[4].starts_with("\u{2506} y"), "{rows:?}");
        assert!(rows.last().unwrap().contains("[y]es"), "{rows:?}");
    }

    #[test]
    fn a_long_input_is_kept_whole_for_the_pager_and_says_so_without_cutting_silently() {
        let lines = many(40);
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let p = ask(&refs, true, OFF, 80);
        let full = p.full.as_deref().expect("more than a screenful comes with the whole of it");
        for line in &lines {
            assert!(full.contains(line.as_str()), "{line}");
        }
        assert!(!full.contains("[y]es"), "the choices are not in the paged text");
        assert!(p.text.contains("more rows (40 lines in all): the pager above showed all of it"), "{}", p.text);
        assert!(p.text.lines().count() < 15 && p.text.contains("[y]es") && !p.text.contains("line-30"), "{}", p.text);
        assert!(p.tail.contains("[y]es") && !p.tail.contains("echo"), "{}", p.tail);
        // Exactly at the limit nothing is held back.
        let twenty = many(MAX_INLINE_ROWS);
        let refs: Vec<&str> = twenty.iter().map(String::as_str).collect();
        let p = ask(&refs, true, OFF, 80);
        assert!(p.full.is_none() && p.text.contains("line-20"), "{}", p.text);
    }

    #[test]
    fn options_of_the_call_are_printed_apart_from_the_input() {
        let lines = vec!["ls".to_string()];
        let notes = vec!["dangerouslyDisableSandbox=true".to_string()];
        let p = permission_prompt(&Question { tool: "Bash", notes: &notes, lines: &lines, always: false, note: None, project_dir: Path::new(PROJECT), style: OFF, width: 100 });
        assert_eq!(rows_of(&p)[..3], ["Allow Bash:", "options: dangerouslyDisableSandbox=true", "\u{2502} ls"]);
    }

    #[test]
    fn the_note_goes_under_the_choices_and_is_wrapped() {
        let lines = vec!["ls".to_string()];
        let note = "can't remember this one: it runs other commands and then some more words to make it long";
        let p = permission_prompt(&Question { tool: "Bash", notes: &[], lines: &lines, always: false, note: Some(note), project_dir: Path::new(PROJECT), style: OFF, width: 50 });
        let rows = rows_of(&p);
        assert_eq!(rows[0], "Allow Bash: ls?  [y]es  [n]o");
        assert!(rows.len() >= 3 && rows.iter().all(|r| display_width(r) < 50), "{rows:?}");
        assert_eq!(rows[1..].concat(), note, "wrapped by columns, no character lost");
    }

    #[test]
    fn the_question_is_plain_text_and_styled_only_when_asked() {
        let p = ask(&["ls"], true, ON, 100);
        assert!(p.text.contains("\x1b[1mAllow Bash: \x1b[0m") && p.text.contains("\x1b[36m[y]\x1b[0m"), "{:?}", p.text);
        assert_eq!(strip_ansi(&p.text), "Allow Bash: ls?  [y]es  [n]o  [a]lways in this project");
        no_esc(&ask(&["ls", "pwd"], true, OFF, 100).text);
    }

    #[test]
    fn answered_questions_and_the_non_interactive_hint_read_plainly() {
        use crate::backend::Decision;
        assert_eq!(permission_answered(Some(&Decision::Allow), None, OFF), "✓ allowed");
        assert_eq!(
            permission_answered(Some(&Decision::AllowAlways), Some("Bash(ls:*)"), OFF),
            "✓ allowed, and always in this project: Bash(ls:*)"
        );
        assert_eq!(permission_answered(Some(&Decision::Deny { reason: "x".into() }), None, OFF), "✗ denied");
        assert_eq!(permission_answered(None, None, OFF), "■ stopped");
        let hint = denied_without_asking("Bash");
        assert!(hint.starts_with("✗ denied Bash") && hint.contains("CODE_ALLOWED_TOOLS") && hint.contains("/permissions"), "{hint}");
    }

    #[test]
    fn the_permissions_screen_lists_rules_folders_user_rules_and_problems() {
        let l = crate::permissions::Listing {
            local_path: "/proj/.claude/settings.local.json".into(),
            allow: vec!["Bash(cargo test:*)".into()],
            directories: vec!["/opt/data".into()],
            user_path: Some("/home/u/.claude/settings.json".into()),
            user_allow: vec!["Bash(git status:*)".into()],
            problems: vec!["/x: not valid JSON".into()],
        };
        let text = permissions_text(&l, OFF, Path::new("/proj"));
        for want in ["Permissions", ".claude/settings.local.json", "    Bash(cargo test:*)", "additional folders", "    /opt/data",
                     "read only", "    Bash(git status:*)", "⚠ /x: not valid JSON", "Codex, Gemini and Grok decide"] {
            assert!(text.contains(want), "{want}: {text}");
        }
        let empty = permissions_text(&crate::permissions::Listing { allow: vec![], directories: vec![], user_allow: vec![], problems: vec![], ..l }, OFF, Path::new("/proj"));
        assert!(empty.contains("none yet") && !empty.contains("additional folders") && !empty.contains("read only"), "{empty}");
    }

    #[test]
    fn permissions_is_in_the_command_table_help_and_completion() {
        assert!(command_names().contains(&"/permissions"));
        assert!(help_text(OFF).contains("/permissions"));
        assert_eq!(command_usage("/permissions").as_deref(), Some("/permissions [allow|remove|dir <x>]"));
        let c = complete("/permissions ", &CompleteCtx::default());
        assert_eq!(c.iter().map(|c| c.replacement.as_str()).collect::<Vec<_>>(), ["allow", "remove", "dir", "list"]);
    }
}
