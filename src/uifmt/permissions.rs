//! The tool-approval question, its answer line, the "denied without asking" hint and the
//! `/permissions` screen.

use std::path::Path;

use super::changes::relativize_paths;
use super::style::Style;
use super::text::one_line;
use super::width::{clamp_width, display_width, truncate_to_width};

/// Columns the summary of an approval question must keep before the choices move to a
/// line of their own.
const MIN_SUMMARY_COLUMNS: usize = 12;

/// The question for one tool call: `Allow Bash: cargo test --quiet? [y]es  [n]o  [a]lways
/// in this project`. `summary` is cut to the width (never wrapped, never hidden by the
/// choices) and shown with paths inside the project relative; when the width leaves it
/// less than a few columns the choices go on the next line. `always` is whether a rule
/// could be saved, which decides if that choice is offered.
pub fn permission_prompt(tool: &str, summary: &str, always: bool, project_dir: &Path, style: Style, width: usize) -> String {
    let width = clamp_width(width).saturating_sub(1);
    let choices_plain = if always { "[y]es  [n]o  [a]lways in this project" } else { "[y]es  [n]o" };
    // The key letter in each choice is the one to press: `[y]es` is cyan, `es` dim.
    let choices = choices_plain
        .split("  ")
        .map(|choice| match choice.split_once(']') {
            Some((key, rest)) => format!("{}{}", style.cyan(&format!("{key}]")), style.dim(rest)),
            None => choice.to_string(),
        })
        .collect::<Vec<_>>()
        .join("  ");
    let head = format!("Allow {}: ", one_line(tool));
    let summary = relativize_paths(&one_line(summary), project_dir);
    // The head, the `?`, two spaces and the choices; what is left is the summary's.
    let fixed = display_width(&head) + 1 + 2 + display_width(choices_plain);
    if width.saturating_sub(fixed) >= MIN_SUMMARY_COLUMNS {
        let shown = truncate_to_width(&summary, width - fixed);
        format!("{}{shown}{}  {choices}", style.bold(&head), style.bold("?"))
    } else {
        let shown = truncate_to_width(&summary, width.saturating_sub(display_width(&head) + 1));
        format!("{}{shown}{}\n{choices}", style.bold(&head), style.bold("?"))
    }
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

    #[test]
    fn the_approval_question_has_the_tool_the_command_and_the_three_choices() {
        let p = Path::new("/proj");
        assert_eq!(
            permission_prompt("Bash", "cargo test --quiet", true, p, OFF, 100),
            "Allow Bash: cargo test --quiet?  [y]es  [n]o  [a]lways in this project"
        );
        assert_eq!(permission_prompt("Bash", "x", false, p, OFF, 100), "Allow Bash: x?  [y]es  [n]o");
        // Paths in the project are shown relative, like tool lines.
        assert!(permission_prompt("Write", "/proj/src/a.rs (12 bytes)", true, p, OFF, 100).contains("Allow Write: src/a.rs (12 bytes)?"));
    }

    #[test]
    fn the_approval_question_fits_the_width_without_hiding_the_choices() {
        let p = Path::new("/proj");
        let long = "x".repeat(300);
        // The choices alone are 38 columns, so the question needs at least 40.
        for width in [40usize, 50, 60, 80, 120] {
            let q = permission_prompt("Bash", &long, true, p, OFF, width);
            for line in q.lines() {
                assert!(display_width(line) < width.max(MIN_WIDTH), "width {width}: {line:?}");
            }
            assert!(q.contains("[y]es") && q.contains("[n]o") && q.contains("[a]lways"), "{q}");
            assert!(q.contains('…'));
        }
        // Wide enough: one line. Narrow: the choices move to a second line.
        assert_eq!(permission_prompt("Bash", &long, true, p, OFF, 100).lines().count(), 1);
        assert_eq!(permission_prompt("Bash", &long, true, p, OFF, 50).lines().count(), 2);
    }

    #[test]
    fn the_approval_question_is_one_clean_line_of_text_and_styled_only_when_asked() {
        let p = Path::new("/proj");
        let q = permission_prompt("Bash", "echo a\n\x1b[2Jb\x07", true, p, OFF, 100);
        assert_eq!(q, "Allow Bash: echo a b?  [y]es  [n]o  [a]lways in this project");
        no_esc(&q);
        let q = permission_prompt("Bash", "ls", true, p, ON, 100);
        assert!(q.contains("\x1b[1mAllow Bash: \x1b[0m") && q.contains("\x1b[36m[y]\x1b[0m"), "{q:?}");
        assert_eq!(strip_ansi(&q), "Allow Bash: ls?  [y]es  [n]o  [a]lways in this project");
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
