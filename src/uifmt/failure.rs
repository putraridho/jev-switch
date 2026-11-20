//! Failures in words: the inline failover note, the fatal error block, and the message
//! for a turn nothing could take. The wording of each failure kind lives in one place.

use crate::backend::{ErrorKind, ProviderId};

use super::provider_view::{AvailRow, ProviderStatus};
use super::style::{mark_code, Style, Theme};
use super::text::{capitalised, one_line};
use super::width::truncate_to_width;

/// What one failure kind says, in each place it is shown. One function per kind below
/// fills it in, so the wording of a kind lives in one place.
struct KindText {
    /// The inline sentence for [`failover_note`].
    inline: String,
    /// What to do next, in one clause, shown after the sentence.
    hint: Option<String>,
    /// The heading of an [`error_block`].
    block_title: &'static str,
    /// The "Try:" list.
    steps: Vec<String>,
}

/// A reset time worth printing: the CLI's own wording, not an epoch number.
fn reset_label(resets_at: Option<&str>) -> Option<String> {
    let r = one_line(resets_at?);
    (!r.is_empty() && !r.chars().all(|c| c.is_ascii_digit())).then_some(r)
}

/// How to log in to `provider`, from the one table in [`crate::doctor`].
fn login_hint(provider: &str) -> String {
    match provider.parse::<ProviderId>() {
        Ok(p) => crate::providers::login_instruction(p),
        Err(_) => format!("log in to {provider}"),
    }
}

fn unusable_here_hint(provider: &str) -> String {
    match provider {
        "codex" => "it needs a git repository here (run `git init`, or work in a repo)".into(),
        "gemini" => "it needs a trusted folder (run `gemini` here once and trust it)".into(),
        _ => "it refuses to run in this directory".into(),
    }
}

fn quota_text(resets_at: Option<&str>, provider: &str, th: &Theme) -> KindText {
    let when = reset_label(resets_at);
    let inline = match &when {
        Some(w) => format!("{} hit its limit (resets {w})", th.name(provider)),
        None => format!("{} hit its limit", th.name(provider)),
    };
    let wait = match &when {
        Some(w) => format!("Wait for the limit to reset ({w}) and send the turn again"),
        None => "Wait for the limit to reset, then send the turn again".into(),
    };
    KindText {
        inline,
        hint: None,
        block_title: "Usage limit reached",
        steps: vec![wait, "Use another provider now: `/model auto` or `/model <provider>`".into(), "See what is ready: `/providers`".into()],
    }
}

fn entitlement_text(provider: &str, model: &str, th: &Theme) -> KindText {
    KindText {
        inline: format!("{}:{model} is not on your plan", th.name(provider)),
        hint: None,
        block_title: "Model not on your plan",
        steps: vec![
            "Let the router pick a model your plan includes: `/model auto`".into(),
            format!("Check what {provider} offers you: `/providers`"),
        ],
    }
}

fn auth_text(provider: &str, th: &Theme) -> KindText {
    let hint = login_hint(provider);
    let need = if provider == "deepseek" { "needs a valid API key" } else { "needs a login" };
    KindText {
        inline: format!("{} {need}", th.name(provider)),
        steps: vec![capitalised(&hint), "Then check it with `/providers refresh`".into()],
        hint: Some(hint),
        block_title: "Login needed",
    }
}

fn empty_text(provider: &str, model: &str, th: &Theme) -> KindText {
    KindText {
        inline: format!("{}:{model} gave an empty answer", th.name(provider)),
        hint: None,
        block_title: "Empty answer",
        steps: vec!["Send it again".into(), "Or pick another model: `/model <provider>`".into()],
    }
}

fn stalled_text(provider: &str, model: &str, th: &Theme) -> KindText {
    KindText {
        inline: format!("{}:{model} stopped answering", th.name(provider)),
        hint: None,
        block_title: "Stopped answering",
        steps: vec![
            "Send it again, or split the task into smaller steps".into(),
            "Raise `CODE_IDLE_TIMEOUT_SECS` if the work is just slow".into(),
        ],
    }
}

fn unusable_here_text(provider: &str, th: &Theme) -> KindText {
    let hint = unusable_here_hint(provider);
    KindText {
        inline: format!("{} can't run in this directory", th.name(provider)),
        steps: vec![capitalised(&hint), "Or use another provider: `/model auto`".into()],
        hint: Some(hint),
        block_title: "Can't run in this directory",
    }
}

fn other_text(provider: &str, model: &str, th: &Theme) -> KindText {
    KindText {
        inline: format!("{}:{model} failed", th.name(provider)),
        hint: None,
        block_title: "The turn failed",
        steps: vec!["Send it again".into(), "If it keeps failing, check the setup: `jev-code doctor`".into()],
    }
}

fn kind_text(kind: &ErrorKind, provider: &str, model: &str, th: &Theme) -> KindText {
    match kind {
        ErrorKind::Quota { resets_at } => quota_text(resets_at.as_deref(), provider, th),
        ErrorKind::NotEntitled => entitlement_text(provider, model, th),
        ErrorKind::Auth => auth_text(provider, th),
        ErrorKind::Empty => empty_text(provider, model, th),
        ErrorKind::Stalled => stalled_text(provider, model, th),
        ErrorKind::UnusableHere => unusable_here_text(provider, th),
        ErrorKind::Other => other_text(provider, model, th),
    }
}

/// The "Try:" steps that fit `kind` for `provider`, for [`error_block`].
pub fn next_steps_for(kind: &ErrorKind, provider: &str) -> Vec<String> {
    kind_text(kind, provider, "", &Theme::new(Style::off())).steps
}

/// The note printed when a model failed and the turn moves on:
/// `⚠ claude hit its limit (resets 5pm) → trying codex · gpt-6.1-sol`, or, with nothing
/// to move to (`to` is `None`), just the problem and what to do:
/// `⚠ codex needs a login: run `codex login``. `discarded` adds
/// `(partial output discarded)` for an attempt that had already shown output.
pub fn failover_note(
    from_provider: &str,
    model: &str,
    kind: &ErrorKind,
    to: Option<(&str, &str)>,
    discarded: bool,
    style: Style,
) -> String {
    let th = Theme::new(style);
    let k = kind_text(kind, from_provider, model, &th);
    let mut s = format!("{} {}", style.yellow("⚠"), k.inline);
    if let Some(h) = &k.hint {
        s.push_str(": ");
        s.push_str(&mark_code(h, style));
    }
    if let Some((p, m)) = to {
        s.push_str(&format!(" → trying {} · {m}", th.name(p)));
    }
    if discarded {
        s.push(' ');
        s.push_str(&style.dim("(partial output discarded)"));
    }
    s
}

const MAX_DETAIL_LINES: usize = 3;
const MAX_DETAIL_COLS: usize = 160;

/// Up to three trimmed, plain lines of a CLI message.
fn detail_lines(message: &str) -> Vec<String> {
    let msg = message.trim();
    let msg = msg.strip_prefix("[error]").map_or(msg, str::trim_start);
    let all: Vec<String> = msg.split('\n').map(one_line).filter(|l| !l.is_empty()).collect();
    let more = all.len() > MAX_DETAIL_LINES;
    let mut lines: Vec<String> = all.into_iter().take(MAX_DETAIL_LINES).map(|l| truncate_to_width(&l, MAX_DETAIL_COLS)).collect();
    if more {
        if let Some(last) = lines.last_mut() {
            last.push_str(" …");
        }
    }
    lines
}

/// A title, dim detail lines and a "Try:" list: the shape of every fatal message.
pub(super) fn block(title: &str, details: &[String], steps: &[String], style: Style) -> String {
    let mut lines = vec![style.paint("1;31", &format!("✗ {title}"))];
    lines.extend(details.iter().map(|d| format!("  {}", style.dim(d))));
    if !steps.is_empty() {
        lines.push(format!("  {}", style.bold("Try:")));
        lines.extend(steps.iter().map(|s| format!("    • {}", mark_code(s, style))));
    }
    lines.join("\n")
}

/// A fatal turn error: a red title for the kind, the CLI's message dimmed (at most 3
/// lines, trimmed, control characters removed) and the steps to try (see
/// [`next_steps_for`]).
pub fn error_block(kind: &ErrorKind, message: &str, next_steps: &[String], style: Style) -> String {
    let title = kind_text(kind, "", "", &Theme::new(Style::off())).block_title;
    block(title, &detail_lines(message), next_steps, style)
}

/// A boxed message in the shape of every fatal message: a red title, dim detail lines and
/// a "Try:" list of steps. For callers that have their own wording.
pub fn problem_block(title: &str, details: &[String], steps: &[String], style: Style) -> String {
    block(title, details, steps, style)
}

/// Why nothing could run, plainly, and the fix: which providers are ready, why each
/// other one is not, and the commands that help (`/providers`, `jev-code doctor`).
pub fn no_route_message(task: &str, rows: &[AvailRow], style: Style) -> String {
    let title = if task.trim().is_empty() {
        "Nothing can take this turn".to_string()
    } else {
        format!("Nothing can take this {} turn", one_line(task))
    };
    let ready: Vec<&str> = rows
        .iter()
        .filter(|r| r.status == ProviderStatus::Ready && r.cooldown.is_none())
        .map(|r| r.provider.as_str())
        .collect();
    let mut details = vec![if ready.is_empty() {
        "No provider is ready to run it.".to_string()
    } else {
        format!("Ready now, but none could take it: {}", ready.join(", "))
    }];
    let mut steps = vec!["See why each one can't run: `/providers`".to_string()];
    let mut fixable = false;
    for r in rows {
        let name = &r.provider;
        match (&r.cooldown, r.status) {
            (Some(left), _) => {
                details.push(format!("{name}: cooling down after a usage limit ({left} left)"));
                steps.push(format!("Wait for {name}'s limit to reset, or use another provider"));
            }
            (None, ProviderStatus::Ready) => {}
            (None, ProviderStatus::NotLoggedIn) => {
                details.push(format!("{name}: not logged in"));
                steps.push(capitalised(&login_hint(name)));
                fixable = true;
            }
            (None, ProviderStatus::NoKey) => {
                details.push(format!("{name}: no API key"));
                steps.push(capitalised(&login_hint(name)));
                fixable = true;
            }
            (None, ProviderStatus::NotInstalled) => details.push(format!("{name}: not installed")),
            (None, ProviderStatus::Unknown) => details.push(format!("{name}: could not be checked")),
        }
    }
    if task.trim() == "security" {
        details.push("Security work never goes to deepseek or grok.".to_string());
    }
    if fixable {
        steps.push("Then run `/providers refresh` and send it again".to_string());
    }
    steps.push("Check the whole setup: `jev-code doctor`".to_string());
    block(&title, &details, &steps, style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::{strip_ansi, display_width};
    use crate::uifmt::testutil::*;
    use crate::doctor::login_steps;
    use crate::providers::{install_hint, login_command, login_instruction, ALL_PROVIDERS};

    #[test]
    fn doctor_steps_and_messages_use_the_same_login_command() {
        for p in ALL_PROVIDERS {
            let steps = login_steps(p, "~/.jev-router.env").join("\n");
            let sentence = login_instruction(p);
            match login_command(p) {
                Some(cmd) => {
                    assert!(steps.contains(cmd), "{p}: {steps}");
                    assert!(sentence.contains(&format!("`{cmd}`")), "{p}: {sentence}");
                    // The uifmt wording is built from the same sentence.
                    assert_eq!(crate::uifmt::next_steps_for(&crate::backend::ErrorKind::Auth, p.as_str())[0].to_lowercase(), sentence.to_lowercase());
                }
                None => {
                    assert!(steps.contains("DEEPSEEK_API_KEY") && sentence.contains("DEEPSEEK_API_KEY"));
                }
            }
            assert!(!install_hint(p).is_empty());
        }
    }

    /// (kind, from provider, model, where to, partial output discarded, expected note)
    type Case<'a> = (ErrorKind, &'a str, &'a str, Option<(&'a str, &'a str)>, bool, &'a str);

    #[test]
    fn failover_notes_per_kind() {
        let to = Some(("codex", "gpt-6.1-sol"));
        let cases: Vec<Case> = vec![
            (quota(), "claude", "opus", to, false, "⚠ claude hit its limit (resets 5pm) → trying codex · gpt-6.1-sol"),
            (ErrorKind::Quota { resets_at: None }, "claude", "opus", to, false, "⚠ claude hit its limit → trying codex · gpt-6.1-sol"),
            // an epoch number is not a time anyone can read
            (ErrorKind::Quota { resets_at: Some("1794853451".into()) }, "claude", "opus", None, false, "⚠ claude hit its limit"),
            (ErrorKind::NotEntitled, "claude", "opus", to, false, "⚠ claude:opus is not on your plan → trying codex · gpt-6.1-sol"),
            (ErrorKind::Auth, "codex", "gpt", None, false, "⚠ codex needs a login: run `codex login`"),
            (ErrorKind::Auth, "claude", "x", Some(("codex", "g")), false, "⚠ claude needs a login: run `claude auth login` → trying codex · g"),
            (ErrorKind::Auth, "deepseek", "x", None, false, "⚠ deepseek needs a valid API key: add `DEEPSEEK_API_KEY` to ~/.jev-router.env"),
            (ErrorKind::Empty, "codex", "gpt", to, false, "⚠ codex:gpt gave an empty answer → trying codex · gpt-6.1-sol"),
            (ErrorKind::Stalled, "codex", "gpt", to, false, "⚠ codex:gpt stopped answering → trying codex · gpt-6.1-sol"),
            (ErrorKind::UnusableHere, "codex", "gpt", Some(("claude", "sonnet")), false, "⚠ codex can't run in this directory: it needs a git repository here (run `git init`, or work in a repo) → trying claude · sonnet"),
            (ErrorKind::UnusableHere, "gemini", "x", None, false, "⚠ gemini can't run in this directory: it needs a trusted folder (run `gemini` here once and trust it)"),
            (ErrorKind::Other, "grok", "g5", Some(("claude", "sonnet")), false, "⚠ grok:g5 failed → trying claude · sonnet"),
            (quota(), "claude", "opus", to, true, "⚠ claude hit its limit (resets 5pm) → trying codex · gpt-6.1-sol (partial output discarded)"),
        ];
        for (kind, from, model, to, discarded, want) in cases {
            assert_eq!(failover_note(from, model, &kind, to, discarded, OFF), want, "{kind:?}");
        }
    }

    #[test]
    fn failover_note_styles_are_additive() {
        for kind in all_kinds() {
            for discarded in [false, true] {
                let styled = failover_note("claude", "opus", &kind, Some(("codex", "g")), discarded, ON);
                let plain = failover_note("claude", "opus", &kind, Some(("codex", "g")), discarded, OFF);
                assert_eq!(strip_ansi(&styled), plain);
                no_esc(&plain);
            }
        }
        let s = failover_note("codex", "g", &ErrorKind::Auth, None, false, ON);
        assert!(s.contains("\x1b[36m`codex login`\x1b[0m"), "{s:?}");
        assert!(s.starts_with("\x1b[33m⚠\x1b[0m"));
    }

    #[test]
    fn error_blocks() {
        let steps = next_steps_for(&ErrorKind::Auth, "codex");
        let s = error_block(&ErrorKind::Auth, "[error] codex gpt failed: 401 Unauthorized\n\n  token expired  \nthird\nfourth", &steps, OFF);
        assert_eq!(
            s,
            "✗ Login needed\n  codex gpt failed: 401 Unauthorized\n  token expired\n  third …\n  Try:\n    • Run `codex login`\n    • Then check it with `/providers refresh`"
        );
        // no steps, no "Try:"
        assert_eq!(error_block(&ErrorKind::Other, "boom", &[], OFF), "✗ The turn failed\n  boom");
        // empty message: just the title
        assert_eq!(error_block(&ErrorKind::Empty, "  \n ", &[], OFF), "✗ Empty answer");
    }

    #[test]
    fn error_block_is_safe_with_hostile_messages() {
        let msg = "\x1b[2J\x1b]0;pwned\x07bad\r\nline\x08two";
        for style in [ON, OFF] {
            let s = error_block(&ErrorKind::Other, msg, &[], style);
            let body: Vec<&str> = s.split('\n').skip(1).collect();
            for l in body {
                assert!(!strip_ansi(l).contains(|c: char| c.is_control()), "{l:?}");
            }
        }
        no_esc(&error_block(&ErrorKind::Other, msg, &[], OFF));
        let long = "x".repeat(500);
        let s = error_block(&ErrorKind::Other, &long, &[], OFF);
        assert!(display_width(s.split('\n').nth(1).unwrap()) <= MAX_DETAIL_COLS + 2);
    }

    #[test]
    fn every_kind_has_steps_and_a_title() {
        for kind in all_kinds() {
            let steps = next_steps_for(&kind, "codex");
            assert!(!steps.is_empty(), "{kind:?}");
            let s = error_block(&kind, "msg", &steps, OFF);
            no_esc(&s);
            assert!(s.starts_with("✗ "));
            assert!(s.contains("Try:"));
        }
    }

    #[test]
    fn no_route_explains_and_fixes() {
        let s = no_route_message("feature", &rows(), OFF);
        assert_eq!(
            s,
            "✗ Nothing can take this feature turn\n  No provider is ready to run it.\n  claude: cooling down after a usage limit (2h 10m left)\n  codex: not logged in\n  gemini: not installed\n  deepseek: no API key\n  grok: could not be checked\n  Try:\n    • See why each one can't run: `/providers`\n    • Wait for claude's limit to reset, or use another provider\n    • Run `codex login`\n    • Add `DEEPSEEK_API_KEY` to ~/.jev-router.env\n    • Then run `/providers refresh` and send it again\n    • Check the whole setup: `jev-code doctor`"
        );
        let sec = no_route_message("security", &rows(), OFF);
        assert!(sec.contains("Security work never goes to deepseek or grok."));
        assert!(!s.contains("Security work"));
        let none = no_route_message("", &[], OFF);
        assert!(none.starts_with("✗ Nothing can take this turn\n  No provider is ready"));
        assert!(none.contains("/providers") && none.contains("jev-code doctor"));
        // a ready provider that still could not take it (it failed this turn)
        let ready = vec![AvailRow::new("codex", ProviderStatus::Ready)];
        assert!(no_route_message("docs", &ready, OFF).contains("Ready now, but none could take it: codex"));
    }

    #[test]
    fn a_problem_block_has_a_title_details_and_steps() {
        let s = problem_block("Title", &["why".to_string()], &["Do `x`".to_string()], OFF);
        assert_eq!(s, "✗ Title\n  why\n  Try:\n    • Do `x`");
    }
}
