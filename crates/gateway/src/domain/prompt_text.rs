//! Reading what the user typed: the words of a prompt worth judging, and the messages
//! Claude Code puts in the user slot that nobody typed.

/// Messages Claude Code puts in the user slot that the user never typed: a finished
/// subagent's report, and the notice that a permission was granted. Routing them as
/// prompts sent every hand-back to Opus on the security keyword rule (169 of 169 in
/// the logs) and reset the step-down counters, so a session that delegated never left
/// Opus. They continue the current turn on whatever model it is already on.
pub fn is_harness_message(prompt: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "Another Claude session sent a message:",
        "Permission granted for this exact command only",
    ];
    let p = prompt.trim_start();
    PREFIXES.iter().any(|x| p.starts_with(x))
}

/// Background-task and subagent notices that Claude Code delivers as prompts. Kept out
/// of `is_harness_message` so routing is unchanged; the orchestrate check skips them.
pub fn is_notification(prompt: &str) -> bool {
    let p = prompt.trim_start();
    p.starts_with("<task-notification") || p.starts_with("<agent-message")
}

pub fn strip_reminders(text: &str) -> String {
    strip_tag(text, "system-reminder")
}

/// `text` without every `<tag>…</tag>` block. An unclosed block runs to the end
/// (prompts are cut at 4,000 characters, so a long one may lose its closing tag).
fn strip_tag(text: &str, tag: &str) -> String {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        out.push_str(&rest[..start]);
        match rest[start..].find(&close) {
            Some(end) => rest = &rest[start + end + close.len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The text inside the first `<tag>…</tag>`, if there is one.
fn tag_text<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&format!("</{tag}>"))?;
    Some(&text[start..start + end])
}

/// What the user actually typed in this prompt, for Jev to judge. None when there is
/// nothing of theirs: a background-task or subagent notice, a local command's echo
/// (`/cockpit`, `/clear`) or the output of a `!` shell command. A skill or custom
/// slash command counts as its name and arguments (`/better-ui wdyt so far?`), not
/// the skill text Claude Code appends. Pasted text is the user's and is kept.
pub fn user_words(prompt: &str) -> Option<String> {
    if is_notification(prompt) {
        return None;
    }
    let local_command = ["<local-command-caveat>", "<local-command-stdout>", "<local-command-stderr>"]
        .iter()
        .any(|t| prompt.contains(t));
    let mut text = prompt.to_string();
    for tag in ["local-command-caveat", "local-command-stdout", "local-command-stderr", "bash-input", "bash-stdout", "bash-stderr"] {
        text = strip_tag(&text, tag);
    }
    if let Some(name) = tag_text(&text, "command-name").map(|n| n.trim().to_string()) {
        if local_command {
            for tag in ["command-name", "command-message", "command-args"] {
                text = strip_tag(&text, tag);
            }
        } else {
            let args = tag_text(&text, "command-args").unwrap_or("").trim();
            text = format!("{name} {args}");
        }
    }
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_are_not_orchestrate_candidates() {
        assert!(is_notification("<task-notification>\n<task-id>a1</task-id>"));
        assert!(is_notification("  <agent-message from=\"a\">report"));
        assert!(!is_notification("why does the <task-notification> hook fire twice?"));
    }

    #[test]
    fn only_the_users_own_words_are_judged() {
        let caveat = "<local-command-caveat>The command below was run directly in Claude Code, not sent to you as a request.</local-command-caveat>";
        let echo = "<command-name>/cockpit</command-name>\n            <command-message>cockpit</command-message>\n            <command-args></command-args>\n\n<local-command-stdout>mech-hud: MECH-OS // COCKPIT ONLINE</local-command-stdout>";
        // A local command's echo alone: nothing to judge.
        assert_eq!(user_words(&format!("{caveat}\n{echo}")), None);
        assert_eq!(user_words(echo), None);
        // Echo followed by what the user typed: just their words.
        assert_eq!(
            user_words(&format!("{caveat}\n{echo}\nbefore start, think deeply about the execution")).as_deref(),
            Some("before start, think deeply about the execution")
        );
        // A skill command: its name and arguments, not the skill text.
        let skill = "<command-message>better-ui</command-message>\n<command-name>/better-ui</command-name>\n<command-args>wdyt of this implementation so far?</command-args>\n\nBase directory for this skill: /home/u/.claude/skills/better-ui\n# Better UI\nPolish the interface...";
        assert_eq!(user_words(skill).as_deref(), Some("/better-ui wdyt of this implementation so far?"));
        // Output of a `!` command is not a request.
        assert_eq!(user_words("<bash-input>cargo test</bash-input>\n\n<bash-stdout>ok</bash-stdout><bash-stderr></bash-stderr>"), None);
        assert_eq!(user_words("<bash-input>sudo pacman -Syu</bash-input>\n<bash-stdout>:: Proceed? [Y/n] Y\n:: Retrieving packages"), None, "unclosed (cut) output");
        // Notifications are never judged.
        assert_eq!(user_words("<task-notification>\n<task-id>a1</task-id>\n<status>completed</status>"), None);
        // Pasted text and ordinary prompts are the user's own.
        let paste = "<pasted_content id=\"9462\">\nerror[E0308]: mismatched types\n</pasted_content>";
        assert_eq!(user_words(paste).as_deref(), Some(paste));
        assert_eq!(user_words("why does the <task-notification> hook fire twice?").as_deref(), Some("why does the <task-notification> hook fire twice?"));
        assert_eq!(user_words("   "), None);
        assert!(!is_notification("add audit logging across the API"));
    }

    #[test]
    fn real_prompts_are_still_routed() {
        for p in [
            "check the upload route for path traversal",
            "another claude session sent a message, why?",
            "is the permission granted for this command in settings.json?",
            "thanks!",
        ] {
            assert!(!is_harness_message(p), "{p}");
        }
    }
}
