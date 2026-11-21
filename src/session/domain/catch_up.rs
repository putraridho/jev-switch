//! Hand-off between providers. Every provider CLI keeps its own session, so when a
//! turn goes to a provider that missed earlier turns (they ran elsewhere) it has
//! to be caught up. The project folder is shared on disk, so the new model can
//! re-read real files; what it lacks is the dialogue and the intent. This module
//! rebuilds exactly that, deterministically and without any LLM call: a short
//! plain-text block (prompts verbatim, replies shrunk, files and commands listed)
//! that the caller prepends to the next prompt.

use super::tool_trail::{commands_run, files_touched};
use super::turn_record::TurnRecord;
use crate::shared::ProviderId;

/// Tokens are estimated as characters / 4 everywhere in the hand-off.
pub(super) const CHARS_PER_TOKEN: usize = 4;

pub(super) fn char_count(s: &str) -> usize {
    s.chars().count()
}

fn estimate_tokens(s: &str) -> usize {
    char_count(s) / CHARS_PER_TOKEN
}
/// How much of the catch-up to keep. One level per step of shrinking; the order of
/// [`LEVELS`] is the order things are given up: older replies first, then older
/// turns' details, and only at the very end the length of prompts.
struct Level {
    /// Reply chars kept for the last missed turn (the one the user is following up).
    last_reply: usize,
    /// Reply chars kept for earlier turns; 0 drops the reply line.
    older_reply: usize,
    /// Whether earlier turns still list files and commands.
    older_details: bool,
    /// Last-resort cut for every prompt; `None` keeps prompts verbatim.
    prompt_cap: Option<usize>,
    git_lines: usize,
}

const LEVELS: &[Level] = &[
    Level { last_reply: 1500, older_reply: 300, older_details: true, prompt_cap: None, git_lines: 40 },
    Level { last_reply: 1500, older_reply: 0, older_details: true, prompt_cap: None, git_lines: 40 },
    Level { last_reply: 1500, older_reply: 0, older_details: false, prompt_cap: None, git_lines: 40 },
    Level { last_reply: 300, older_reply: 0, older_details: false, prompt_cap: Some(400), git_lines: 10 },
];

/// Who the receiving assistant is: `provider/model`, or just the provider when the
/// model is not known.
fn identity(p: ProviderId, model: Option<&str>) -> String {
    match model.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => format!("{p}/{m}"),
        None => p.to_string(),
    }
}

/// The block's header. It must not read as the whole history: the receiving model
/// lived through its own turns and keeps them in its own memory, so it is told that
/// only turns that ran elsewhere are listed.
fn header(you: &str) -> String {
    format!(
        "You are: {you}\n\
         Catch-up: below are ONLY the turns of this conversation that ran on other assistants while you were away \
         (each turn header names the assistant that ran it, as provider/model). \
         Anything you did yourself earlier in this conversation is already in your own memory and is NOT repeated here, \
         so this list is not the whole history. \
         The project folder is shared, so files may have changed since you last looked: re-read a file before editing it. \
         The user's current message follows the line \"Now the user says:\"."
    )
}

/// `s` cut to `n` chars, with an ellipsis when something was removed.
pub(in crate::session) fn truncate_chars(s: &str, n: usize) -> String {
    if char_count(s) <= n {
        return s.to_string();
    }
    let mut cut: String = s.chars().take(n).collect();
    cut.push('…');
    cut
}

/// The first `max` lines of `text`, saying how many were left out.
fn truncate_lines(text: &str, max: usize) -> String {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    if lines.len() <= max {
        return lines.join("\n");
    }
    format!("{}\n... ({} more lines)", lines[..max].join("\n"), lines.len() - max)
}

/// One missed turn. `is_last` turns get the long reply budget and always keep their details.
fn render_turn(index: usize, total: usize, rec: &TurnRecord, is_last: bool, level: &Level) -> String {
    let marker = if rec.ok { "ok" } else { "FAILED" };
    let mut out = format!("[turn {index} of {total}: {}/{}, {marker}]\n", rec.provider, rec.model);
    let prompt = match level.prompt_cap {
        Some(cap) => truncate_chars(&rec.prompt, cap),
        None => rec.prompt.clone(),
    };
    out.push_str(&format!("user: {prompt}\n"));
    let reply_chars = if is_last { level.last_reply } else { level.older_reply };
    if reply_chars > 0 && !rec.reply.trim().is_empty() {
        out.push_str(&format!("assistant: {}\n", truncate_chars(rec.reply.trim(), reply_chars)));
    }
    if is_last || level.older_details {
        for (label, items) in [("files touched", files_touched(&rec.tools)), ("commands run", commands_run(&rec.tools))] {
            if !items.is_empty() {
                out.push_str(&format!("{label}: {}\n", items.join(", ")));
            }
        }
    }
    out
}

/// The whole catch-up block at one level of detail. Pure, so each shrink step is
/// just another call.
fn render(missed: &[TurnRecord], you: &str, git_status: &str, level: &Level) -> String {
    let total = missed.len();
    let mut out = format!("{}\n\n", header(you));
    for (i, rec) in missed.iter().enumerate() {
        out.push_str(&render_turn(i + 1, total, rec, i + 1 == total, level));
        out.push('\n');
    }
    let git = truncate_lines(git_status, level.git_lines);
    let git = if git.is_empty() { "(no changes)".to_string() } else { git };
    out.push_str(&format!("git status:\n{git}\n\nNow the user says:\n"));
    out
}

/// The catch-up for `missed` (never empty), shrunk level by level until it fits
/// `budget_tokens`; the smallest form is returned even when it does not.
pub(super) fn build(missed: &[TurnRecord], p: ProviderId, model: Option<&str>, git_status: &str, budget_tokens: usize) -> String {
    let mut text = String::new();
    for level in LEVELS {
        text = render(missed, &identity(p, model), git_status, level);
        if estimate_tokens(&text) <= budget_tokens {
            break;
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::session::Sessions;


    fn rec(provider: ProviderId, prompt: &str, reply: &str, tools: &[&str], ok: bool) -> TurnRecord {
        TurnRecord {
            prompt: prompt.into(),
            provider,
            model: "m1".into(),
            reply: reply.into(),
            tools: tools.iter().map(|t| t.to_string()).collect(),
            ok,
        }
    }

    fn long_session() -> Sessions {
        let mut s = Sessions::new();
        for i in 0..4 {
            let reply = format!("REPLY{i}{}", i.to_string().repeat(1200));
            s.record(rec(ProviderId::Claude, &format!("PROMPT{i} please"), &reply, &[&format!("Edit src/f{i}.rs")], true));
        }
        s
    }

    #[test]
    fn nothing_missed_gives_none() {
        let mut s = Sessions::new();
        assert_eq!(s.catch_up(ProviderId::Claude, "", 1000), None);
        s.record(rec(ProviderId::Claude, "hi", "hello", &[], true));
        assert_eq!(s.catch_up(ProviderId::Claude, "", 1000), None);
    }

    #[test]
    fn one_missed_turn_has_prompt_file_and_git_status() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "Fix the parser\nin src/a.rs, please", "Done.", &["Edit src/a.rs", "Bash cargo test"], true));
        let text = s.catch_up(ProviderId::Codex, " M src/a.rs", 1000).unwrap();
        assert!(text.contains("Fix the parser\nin src/a.rs, please"));
        assert!(text.contains("files touched: src/a.rs"));
        assert!(text.contains("commands run: cargo test"));
        assert!(text.contains("Done."));
        assert!(text.contains("git status:\n M src/a.rs"));
        assert!(text.contains("ok]"));
        assert!(text.trim_end().ends_with("Now the user says:"));
    }

    #[test]
    fn header_says_only_other_assistants_turns_are_listed_and_never_claims_full_history() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "rename the word", "renamed", &["Edit src/a.rs"], true));
        let text = s.catch_up(ProviderId::Codex, "", 1000).unwrap();
        assert!(text.contains("ONLY the turns"), "{text}");
        assert!(text.contains("other assistants while you were away"));
        assert!(text.contains("provider/model"));
        assert!(text.contains("already in your own memory and is NOT repeated here"));
        assert!(text.contains("shared") && text.contains("re-read"));
        assert!(text.contains("\"Now the user says:\""));
        for old in ["done by other assistants", "other assistants did the work", "The work so far", "You did not see it"] {
            assert!(!text.contains(old), "old phrase {old:?} is back");
        }
    }

    #[test]
    fn header_names_the_receiving_identity() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "p", "r", &[], true));
        let with = s.catch_up_for(ProviderId::Codex, "gpt-6-luna", "", 1000).unwrap();
        assert!(with.starts_with("You are: codex/gpt-6-luna\n"), "{with}");
        // The wrapper has no model, so only the provider is named.
        let without = s.catch_up(ProviderId::Codex, "", 1000).unwrap();
        assert!(without.starts_with("You are: codex\n"), "{without}");
        // Turn headers still name who ran each turn.
        assert!(with.contains("[turn 1 of 1: claude/m1, ok]"));
        // The identity survives the tightest budget.
        let tiny = s.catch_up_for(ProviderId::Codex, "gpt-6-luna", "", 1).unwrap();
        assert!(tiny.contains("You are: codex/gpt-6-luna"));
    }

    #[test]
    fn failed_turn_is_listed_and_marked() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "try the build", "", &["Bash cargo build"], false));
        let text = s.catch_up(ProviderId::Codex, "", 1000).unwrap();
        assert!(text.contains("try the build"));
        assert!(text.contains("FAILED"));
        assert!(text.contains("commands run: cargo build"));
    }

    #[test]
    fn full_budget_keeps_older_replies_short_and_last_reply_long() {
        let text = long_session().catch_up(ProviderId::Codex, "", 100_000).unwrap();
        assert!(text.contains("REPLY0") && text.contains("REPLY1"));
        assert!(text.contains("files touched: src/f0.rs"));
        // Earlier replies are cut to ~300 chars, the last one keeps up to 1500.
        assert!(!text.contains(&"0".repeat(400)));
        assert!(text.contains(&"0".repeat(290)));
        assert!(text.contains(&"3".repeat(1100)));
    }

    #[test]
    fn shrinking_budget_drops_old_replies_before_details_before_prompt_length() {
        let s = long_session();
        let full = s.catch_up(ProviderId::Codex, "", 100_000).unwrap();
        // Just under the full size: older replies go, details stay.
        let no_old_replies = s.catch_up(ProviderId::Codex, "", estimate_tokens(&full) - 1).unwrap();
        assert!(!no_old_replies.contains("REPLY0") && !no_old_replies.contains("REPLY2"));
        assert!(no_old_replies.contains("REPLY3"));
        assert!(no_old_replies.contains("files touched: src/f0.rs"));
        // Tighter: older details go too.
        let tight = s.catch_up(ProviderId::Codex, "", estimate_tokens(&no_old_replies) - 1).unwrap();
        assert!(!tight.contains("src/f0.rs") && !tight.contains("src/f2.rs"));
        assert!(tight.contains("src/f3.rs") && tight.contains("REPLY3"));
        for i in 0..4 {
            assert!(tight.contains(&format!("PROMPT{i} please")));
        }
        assert!(estimate_tokens(&tight) < estimate_tokens(&no_old_replies));
    }

    #[test]
    fn prompts_survive_an_impossible_budget_and_only_then_get_cut() {
        let mut s = Sessions::new();
        let long_prompt = format!("START {} END", "p".repeat(2000));
        s.record(rec(ProviderId::Claude, &long_prompt, "r", &[], true));
        s.record(rec(ProviderId::Claude, "second prompt", "r", &[], true));
        let fits = s.catch_up(ProviderId::Codex, "", 100_000).unwrap();
        assert!(fits.contains(&long_prompt), "verbatim when it fits");
        let tiny = s.catch_up(ProviderId::Codex, "", 1).unwrap();
        assert!(tiny.contains("START ") && tiny.contains("second prompt"));
        assert!(!tiny.contains("END"), "last resort cuts to 400 chars");
        assert!(tiny.contains("Now the user says:"));
    }

    #[test]
    fn git_status_is_cut_to_forty_lines() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "p", "r", &[], true));
        let git: String = (0..60).map(|i| format!(" M file{i}.rs\n")).collect();
        let text = s.catch_up(ProviderId::Codex, &git, 100_000).unwrap();
        assert!(text.contains("file39.rs") && !text.contains("file40.rs"));
        assert!(text.contains("(20 more lines)"));
    }
}
