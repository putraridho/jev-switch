//! Hand-off between providers. Every provider CLI keeps its own session, so when a
//! turn goes to a provider that missed earlier turns (they ran elsewhere) it has
//! to be caught up. The project folder is shared on disk, so the new model can
//! re-read real files; what it lacks is the dialogue and the intent. This module
//! rebuilds exactly that, deterministically and without any LLM call: a short
//! plain-text block (recent prompts, replies shrunk, files and commands listed)
//! that the caller prepends to the next prompt.

use super::tool_trail::{commands_run, files_touched};
use super::turn_record::TurnRecord;
use jev_shared::ProviderId;

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
/// turns' details. If these do not fit, keep a recent window before cutting the
/// latest turn's text.
struct Level {
    /// Reply chars kept for the last missed turn (the one the user is following up).
    last_reply: usize,
    /// Reply chars kept for earlier turns; 0 drops the reply line.
    older_reply: usize,
    /// Whether earlier turns still list files and commands.
    older_details: bool,
    /// Last-resort cut for every prompt; `None` keeps prompts verbatim.
    prompt_cap: Option<usize>,
    detail_cap: Option<usize>,
    git_lines: usize,
}

const LEVELS: &[Level] = &[
    Level { last_reply: 1500, older_reply: 300, older_details: true, prompt_cap: None, detail_cap: None, git_lines: 40 },
    Level { last_reply: 1500, older_reply: 0, older_details: true, prompt_cap: None, detail_cap: None, git_lines: 40 },
    Level { last_reply: 1500, older_reply: 0, older_details: false, prompt_cap: None, detail_cap: None, git_lines: 40 },
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
         that fit this catch-up budget; earlier missed turns may be omitted. \
         (each turn header names the assistant that ran it, as provider/model). \
         Anything you did yourself earlier in this conversation is already in your own memory and is NOT repeated here, \
         so this list is not the whole history. \
         The project folder is shared, so files may have changed since you last looked: re-read a file before editing it. \
         The user's current message follows the line \"Now the user says:\"."
    )
}

/// `s` cut to `n` chars, with an ellipsis when something was removed.
pub(in crate) fn truncate_chars(s: &str, n: usize) -> String {
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
    let model = match level.detail_cap {
        Some(cap) => truncate_chars(&rec.model, cap),
        None => rec.model.clone(),
    };
    let mut out = format!("[turn {index} of {total}: {}/{model}, {marker}]\n", rec.provider);
    let prompt = match level.prompt_cap {
        Some(cap) => truncate_chars(&rec.prompt, cap),
        None => rec.prompt.clone(),
    };
    out.push_str(&format!("user: {prompt}\n"));
    let reply_chars = if is_last { level.last_reply } else { level.older_reply };
    let mut omitted = level.prompt_cap.is_some_and(|cap| char_count(&rec.prompt) > cap)
        || char_count(rec.reply.trim()) > reply_chars
        || level.detail_cap.is_some_and(|cap| char_count(&rec.model) > cap);
    if reply_chars > 0 && !rec.reply.trim().is_empty() {
        out.push_str(&format!("assistant: {}\n", truncate_chars(rec.reply.trim(), reply_chars)));
    }
    for (label, items) in [("files touched", files_touched(&rec.tools)), ("commands run", commands_run(&rec.tools))] {
        if items.is_empty() {
            continue;
        }
        if is_last || level.older_details {
            let details = items.join(", ");
            let details = match level.detail_cap {
                Some(cap) => {
                    omitted |= char_count(&details) > cap;
                    truncate_chars(&details, cap)
                }
                None => details,
            };
            out.push_str(&format!("{label}: {details}\n"));
        } else {
            omitted = true;
        }
    }
    if omitted {
        out.push_str("[text/details truncated or omitted from this turn; canonical session log unchanged]\n");
    }
    out
}

/// The whole catch-up block at one level of detail. Pure, so each shrink step is
/// just another call.
fn render(missed: &[TurnRecord], start: usize, you: &str, git_status: &str, level: &Level) -> String {
    let total = missed.len();
    let mut out = format!("{}\n\n", header(you));
    out.push_str(&provenance(total, start));
    for (i, rec) in missed.iter().enumerate().skip(start) {
        out.push_str(&render_turn(i + 1, total, rec, i + 1 == total, level));
        out.push('\n');
    }
    let git = truncate_lines(git_status, level.git_lines);
    let git = if git.is_empty() { "(no changes)".to_string() } else { git };
    out.push_str(&format!("git status:\n{git}\n\nNow the user says:\n"));
    out
}

fn provenance(total: usize, start: usize) -> String {
    format!("Provenance: {total} missed turns; {start} earlier missed turns omitted; canonical session log unchanged.\n")
}

fn render_latest(missed: &[TurnRecord], p: ProviderId, model: Option<&str>, cap: usize) -> String {
    let total = missed.len();
    let mut out = format!("You are: {}\nCatch-up excerpt from other assistants; re-read shared files before editing.\n", truncate_chars(&identity(p, model), cap));
    out.push_str(&provenance(total, total - 1));
    let level = Level { last_reply: cap, older_reply: 0, older_details: false, prompt_cap: Some(cap), detail_cap: Some(cap), git_lines: 0 };
    out.push_str(&render_turn(total, total, &missed[total - 1], true, &level));
    out.push_str("git status omitted (budget).\nNow the user says:\n");
    out
}

/// The delivered catch-up always fits the estimated budget. Shrink older detail,
/// then the recent window, then the latest turn. Budgets too small for provenance
/// receive an explicit omission marker (even zero tokens can hold `…` under the
/// character estimator). The canonical history and seen state are never changed.
pub(super) fn build(missed: &[TurnRecord], p: ProviderId, model: Option<&str>, git_status: &str, budget_tokens: usize) -> String {
    if missed.is_empty() {
        return String::new();
    }
    let you = identity(p, model);
    for level in LEVELS {
        let text = render(missed, 0, &you, git_status, level);
        if estimate_tokens(&text) <= budget_tokens {
            return text;
        }
    }
    let mut window = missed.len();
    while window > 1 {
        window = (window / 2).max(1);
        let text = render(missed, missed.len() - window, &you, git_status, &LEVELS[0]);
        if estimate_tokens(&text) <= budget_tokens {
            return text;
        }
    }
    let mut cap = budget_tokens.saturating_mul(CHARS_PER_TOKEN);
    loop {
        let text = render_latest(missed, p, model, cap);
        if estimate_tokens(&text) <= budget_tokens {
            return text;
        }
        if cap == 0 {
            break;
        }
        cap /= 2;
    }
    for marker in [
        "Catch-up omitted (budget); canonical session log unchanged; re-read shared files.\nNow the user says:\n",
        "Catch-up omitted; re-read shared files.\nNow the user says:\n",
        "Catch-up omitted; re-read shared files.\n",
        "Omitted\n",
    ] {
        if estimate_tokens(marker) <= budget_tokens {
            return marker.to_string();
        }
    }
    "…\n".to_string()
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
        let tiny = s.catch_up_for(ProviderId::Codex, "gpt-6-luna", "", 1).unwrap();
        assert_eq!(tiny, "…\n");
        assert!(estimate_tokens(&tiny) <= 1);
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
    fn impossible_budget_marks_omission_instead_of_returning_prompts_over_budget() {
        let mut s = Sessions::new();
        let long_prompt = format!("START {} END", "p".repeat(2000));
        s.record(rec(ProviderId::Claude, &long_prompt, "r", &[], true));
        s.record(rec(ProviderId::Claude, "second prompt", "r", &[], true));
        let fits = s.catch_up(ProviderId::Codex, "", 100_000).unwrap();
        assert!(fits.contains(&long_prompt), "verbatim when it fits");
        let tiny = s.catch_up(ProviderId::Codex, "", 1).unwrap();
        assert_eq!(tiny, "…\n");
        assert!(estimate_tokens(&tiny) <= 1);
    }

    #[test]
    fn long_history_keeps_a_bounded_recent_window_without_changing_history_or_seen() {
        let mut session = Sessions::new();
        for index in 0..100 {
            let prompt = format!("INTENT{index:03} {}", "p".repeat(1200));
            session.record(rec(ProviderId::Claude, &prompt, &format!("REPLY{index:03}"), &["Edit src/latest.rs", "Bash cargo test"], index != 99));
        }
        let canonical = session.log().to_vec();
        let missed = session.missed_turns(ProviderId::Codex);
        let text = session.catch_up(ProviderId::Codex, " M src/latest.rs", 10_000).unwrap();
        assert!(estimate_tokens(&text) <= 10_000);
        assert!(!text.contains("INTENT000"));
        assert!(text.contains(&canonical[98].prompt));
        assert!(text.contains(&canonical[99].prompt));
        assert!(text.contains("[turn 100 of 100: claude/m1, FAILED]"));
        assert!(text.contains("REPLY099"));
        assert!(text.contains("files touched: src/latest.rs"));
        assert!(text.contains("commands run: cargo test"));
        assert!(text.contains("git status:\n M src/latest.rs"));
        assert!(text.contains("earlier missed turns omitted; canonical session log unchanged"));
        assert!(text.contains("re-read a file before editing it"));
        assert!(text.trim_end().ends_with("Now the user says:"));
        let shown = text.matches("[turn ").count();
        assert!(text.contains(&provenance(100, 100 - shown)));
        assert_eq!(session.catch_up(ProviderId::Codex, " M src/latest.rs", 10_000).unwrap(), text);
        assert_eq!(session.log(), canonical);
        assert_eq!(session.missed_turns(ProviderId::Codex), missed);
        assert_eq!(session.missed_turns(ProviderId::Claude), 0);
    }

    #[test]
    fn latest_turn_shrinks_with_explicit_provenance_and_preserves_intent_and_details() {
        let mut session = Sessions::new();
        session.record(rec(ProviderId::Claude, "earlier intent", "earlier reply", &[], true));
        session.record(rec(ProviderId::Claude, &format!("LATEST INTENT {}", "p".repeat(8000)), &format!("LATEST REPLY {}", "r".repeat(8000)), &["Edit src/latest.rs", "Bash cargo test"], false));
        let text = session.catch_up_for(ProviderId::Codex, "receiver", " M src/latest.rs", 500).unwrap();
        assert!(estimate_tokens(&text) <= 500);
        assert!(text.starts_with("You are: codex/receiver\n"));
        assert!(text.contains("LATEST INTENT"));
        assert!(text.contains("LATEST REPLY"));
        assert!(text.contains("[turn 2 of 2: claude/m1, FAILED]"));
        assert!(text.contains("src/latest.rs") && text.contains("cargo test"));
        assert!(text.contains(&provenance(2, 1)));
        assert!(text.contains("text/details truncated or omitted from this turn"));
        assert!(text.contains("re-read shared files before editing"));
        assert!(text.trim_end().ends_with("Now the user says:"));
    }

    #[test]
    fn unicode_and_oversized_metadata_stay_bounded_at_every_small_budget() {
        let mut session = Sessions::new();
        let mut latest = rec(ProviderId::Claude, &"修正🦀e\u{301}".repeat(400), &"回答🦀".repeat(400), &[&format!("Edit {}", "路🦀".repeat(400)), &format!("Bash {}", "検🦀".repeat(400))], true);
        latest.model = "模型🦀".repeat(400);
        session.push_unseen(latest);
        let canonical = session.log().to_vec();
        let git = format!(" M {}", "路🦀".repeat(4000));
        for budget in 0..=1000 {
            let text = session.catch_up_for(ProviderId::Codex, &"受信🦀".repeat(400), &git, budget).unwrap();
            assert!(estimate_tokens(&text) <= budget, "budget {budget}, estimate {}", estimate_tokens(&text));
            assert!(!text.is_empty(), "budget {budget} silently discarded history");
            assert!(text.contains("omitted") || text.trim() == "Omitted" || text.trim() == "…", "{text}");
        }
        let text = session.catch_up(ProviderId::Codex, "", 500).unwrap();
        assert!(text.contains("user: 修正🦀e\u{301}"));
        assert!(text.contains("assistant: 回答🦀"));
        assert!(text.contains('…'));
        assert_eq!(session.log(), canonical);
        assert_eq!(session.missed_turns(ProviderId::Codex), 1);
    }

    #[test]
    fn tiny_zero_and_maximum_budgets_are_safe_and_deterministic() {
        let mut session = Sessions::new();
        session.push_unseen(rec(ProviderId::Claude, "intent", "answer", &[], true));
        for budget in [0, 1, 2, 3, 8, 16, 24, 32, 64, 128, usize::MAX] {
            let text = session.catch_up(ProviderId::Codex, "", budget).unwrap();
            assert!(estimate_tokens(&text) <= budget);
            assert!(!text.is_empty());
            assert!(text.ends_with('\n'), "the current prompt must not run into the omission marker");
            assert_eq!(session.catch_up(ProviderId::Codex, "", budget).unwrap(), text);
        }
        assert_eq!(session.catch_up(ProviderId::Codex, "", 0).unwrap(), "…\n");
        assert_eq!(session.missed_turns(ProviderId::Codex), 1);
        assert_eq!(build(&[], ProviderId::Codex, None, "", 0), "");
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
