//! The conversation as the router saw it: the turn log, which provider has seen how much
//! of it, and the CLI session each provider resumes.

use std::collections::HashMap;

use super::catch_up::{self, char_count, CHARS_PER_TOKEN};
use super::turn_record::{TurnRecord, INTERRUPTED_NOTE, UNDO_NOTE};
use jev_shared::ProviderId;

/// The conversation as the router saw it, plus what each provider's own CLI
/// session has lived through. `seen` is a prefix length into `log`: a provider has
/// seen `log[..seen]`. A prefix is enough because a provider that runs a turn is
/// (by the caller's contract) caught up first, so it never has holes.
#[derive(Debug, Clone, Default)]
pub struct Sessions {
    pub( crate) ids: HashMap<ProviderId, String>,
    pub( crate) seen: HashMap<ProviderId, usize>,
    pub( crate) log: Vec<TurnRecord>,
    /// The name the person gave the session (`/rename`), shown in place of its first message.
    pub( crate) name: Option<String> }

fn record_chars(r: &TurnRecord) -> usize {
    char_count(&r.prompt) +
        char_count(&r.reply) +
        r.tools
            .iter()
            .map(|t| char_count(t))
            .sum::<usize>() }

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// The CLI session to resume for `p`, if it has one.
    pub fn session_id(&self, p: ProviderId) -> Option<&str> {
        self.ids.get(&p).map(String::as_str)
    }

    pub fn set_session_id(&mut self, p: ProviderId, id: String) {
        self.ids.insert(p, id);
    }

    /// Append a turn. The provider that ran it lived it, so it is marked as having
    /// seen everything up to and including this turn. This relies on the caller
    /// having caught the provider up (see [`Sessions::mark_seen`]) before running it.
    pub fn record(&mut self, rec: TurnRecord) {
        let p = rec.provider;
        self.log.push(rec);
        self.mark_seen(p);
    }

    /// Append a turn that got NO answer. Unlike [`Sessions::record`] this marks the
    /// provider caught up only if it already was before this turn: a provider that was
    /// still owed earlier turns when its attempt failed (login, stall, wrong folder)
    /// never read them, so it must get them next time. A provider that was current keeps
    /// its place (its own CLI session has seen the prompt).
    pub fn record_failed(&mut self, rec: TurnRecord) {
        let p = rec.provider;
        let was_current = self.missed_turns(p) == 0;
        self.log.push(rec);
        if was_current {
            self.mark_seen(p);
        }
    }

    /// Append a turn the user cancelled. Like [`Sessions::record_undo`] it marks NOBODY as
    /// caught up, not even `provider`, which ran it: that provider's own CLI session was
    /// killed mid-turn and never recorded it, so its next prompt carries the note too.
    pub fn record_interrupted(&mut self, prompt: &str, provider: ProviderId, model: &str, tools: Vec<String>) {
        self.log.push(TurnRecord {
            prompt: prompt.to_string(),
            provider,
            model: model.to_string(),
            reply: INTERRUPTED_NOTE.to_string(),
            tools,
            ok: false,
        });
    }

    /// Append a turn nobody has lived through (history brought from elsewhere): every
    /// provider is owed it, so the first prompt on any of them carries it.
    pub fn push_unseen(&mut self, rec: TurnRecord) {
        self.log.push(rec);
    }

    /// Declare `p` caught up with the whole log. Called after [`Sessions::catch_up`]
    /// has been put in front of a prompt.
    pub fn mark_seen(&mut self, p: ProviderId) {
        self.seen.insert(p, self.log.len());
    }

    /// How many logged turns `p`'s own session has not lived through.
    pub fn missed_turns(&self, p: ProviderId) -> usize {
        self.log.len().saturating_sub(self.seen.get(&p).copied().unwrap_or(0))
    }

    /// The catch-up block for `p`, or `None` when it missed nothing. Ends with
    /// `Now the user says:` so the caller can append the new prompt directly. It is
    /// shrunk step by step until it fits `budget_tokens` (see `catch_up`); if even
    /// the smallest form is over budget it is returned anyway, because losing a
    /// user prompt is worse than a few extra tokens.
    pub fn catch_up(&self, p: ProviderId, git_status: &str, budget_tokens: usize) -> Option<String> {
        self.catch_up_as(p, None, git_status, budget_tokens)
    }

    /// Like [`Sessions::catch_up`], but the header also names the receiving model
    /// (`You are: codex/gpt-6-luna`), so it can tell its own turns from the listed ones.
    pub fn catch_up_for(&self, p: ProviderId, model: &str, git_status: &str, budget_tokens: usize) -> Option<String> {
        self.catch_up_as(p, Some(model), git_status, budget_tokens)
    }

    fn catch_up_as(&self, p: ProviderId, model: Option<&str>, git_status: &str, budget_tokens: usize) -> Option<String> {
        let missed = &self.log[self.log.len() - self.missed_turns(p)..];
        if missed.is_empty() {
            return None;
        }
        Some(catch_up::build(missed, p, model, git_status, budget_tokens))
    }

    /// Rough token size of the whole dialogue at the built-in four characters per token.
    pub fn context_tokens_estimate(&self) -> usize {
        self.context_chars() / CHARS_PER_TOKEN
    }

    /// Characters of the whole dialogue (prompts, replies and tool rows), for estimating its
    /// size in a provider's own tokens.
    pub fn context_chars(&self) -> usize { self
            .log
            .iter()
            .map(record_chars).sum()
    }

    /// Characters of the turns `p`'s own session has not lived through: what a catch-up
    /// for it would be made from (before it is shrunk to its budget).
    pub fn missed_chars(&self, p: ProviderId) -> usize {
        self.log[self.log.len() - self.missed_turns(p)..].iter().map(record_chars)
            .sum()
    }

    /// The logged turns, oldest first (read-only: the session library searches them).
    pub fn log(&self) -> &[TurnRecord] {
        &self.log
    }

    /// Number of logged turns (answered, failed, interrupted, and undo notes).
    pub fn turns(&self) -> usize {
        self.log.len()
    }

    /// Every logged turn, oldest first (for `/replay`).
    pub fn records(&self) -> &[TurnRecord] {
        &self.log
    }

    /// The name given with `/rename`, if any.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Name the session (`None` takes the name away). Whitespace is collapsed.
    pub fn set_name(&mut self, name: Option<&str>) {
        self.name = name.map(|n| n.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|n| !n.is_empty());
    }

    /// What the session is called in a list: its name, else what the person first asked.
    pub fn title(&self) -> Option<String> {
        self.name.clone().or_else(|| self.first_prompt())
    }

    /// What the person first asked, one line: the name of the session in a list.
    pub fn first_prompt(&self) -> Option<String> {
        self.log.iter().map(|r| r.prompt.trim()).find(|p| !p.is_empty()).map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    /// Providers that ran a turn, in the order they first appeared.
    pub fn providers_used(&self) -> Vec<ProviderId> {
        let mut out: Vec<ProviderId> = Vec::new();
        for r in &self.log {
            if !out.contains(&r.provider) {
                out.push(r.provider);
            }
        }
        out
    }

    /// The prompt and reply of the last turn that was answered, for Jev's correction question.
    pub fn last_answered(&self) -> Option<(&str, &str)> {
        self.log.iter().rev().find(|r| r.ok).map(|r| (r.prompt.as_str(), r.reply.as_str()))
    }

    /// Note that the user undid the changes of the previous turn. Unlike
    /// [`Sessions::record_failed`] this marks nobody as caught up, not even the provider
    /// that ran the turn: its own CLI session still believes the files are as it left
    /// them, so its next prompt carries this note and tells it to look again.
    pub fn record_undo(&mut self) {
        let (provider, model) = self.log.last().map_or((ProviderId::Claude, String::new()), |r| (r.provider, r.model.clone()));
        self.log.push(TurnRecord {
            prompt: "/undo".into(),
            provider,
            model,
            reply: UNDO_NOTE.into(),
            tools: Vec::new(),
            ok: false,
        });
    }

    /// Forget everything (`/new`): sessions, what was seen, and the log.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The last `max_turns` turns as `user: ...\nassistant: ...` pairs, newest last,
    /// with replies cut to `reply_chars`. Short context for cheap classifiers that
    /// must tell what a short follow-up like "do it" refers to.
    pub fn recent_context(&self, max_turns: usize, reply_chars: usize) -> String {
        let start = self.log.len().saturating_sub(max_turns);
        self.log[start..]
            .iter()
            .map(|r| format!("user: {}\nassistant: {}", r.prompt, r.reply.chars().take(reply_chars).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;


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

    #[test]
    fn provider_that_ran_the_turn_never_misses_it() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "one", "r1", &[], true));
        s.record(rec(ProviderId::Codex, "two", "r2", &[], true));
        assert_eq!(s.missed_turns(ProviderId::Codex), 0);
        assert_eq!(s.missed_turns(ProviderId::Claude), 1);
        assert_eq!(s.missed_turns(ProviderId::Gemini), 2);
    }

    #[test]
    fn mark_seen_clears_missed_until_the_next_foreign_turn() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "one", "r1", &[], true));
        assert_eq!(s.missed_turns(ProviderId::Codex), 1);
        s.mark_seen(ProviderId::Codex);
        assert_eq!(s.missed_turns(ProviderId::Codex), 0);
        assert_eq!(s.catch_up(ProviderId::Codex, "", 1000), None);
        s.record(rec(ProviderId::Claude, "two", "r2", &[], true));
        let text = s.catch_up(ProviderId::Codex, "", 1000).unwrap();
        assert!(text.contains("two") && !text.contains("user: one"));
        assert!(text.contains("[turn 1 of 1"));
    }

    #[test]
    fn reset_forgets_everything() {
        let mut s = Sessions::new();
        s.set_session_id(ProviderId::Claude, "abc".into());
        s.record(rec(ProviderId::Claude, "p", "r", &[], true));
        assert_eq!(s.session_id(ProviderId::Claude), Some("abc"));
        s.reset();
        assert_eq!(s.session_id(ProviderId::Claude), None);
        assert_eq!(s.missed_turns(ProviderId::Codex), 0);
        assert_eq!(s.context_tokens_estimate(), 0);
        assert_eq!(s.recent_context(3, 100), "");
    }

    #[test]
    fn context_estimate_is_chars_over_four() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, &"a".repeat(40), &"b".repeat(60), &["Edit x"], true));
        assert_eq!(s.context_tokens_estimate(), (40 + 60 + 6) / 4);
    }

    #[test]
    fn recent_context_format_newest_last_and_truncated() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "q1", "a1", &[], true));
        s.record(rec(ProviderId::Codex, "q2", "abcdefgh", &[], true));
        s.record(rec(ProviderId::Claude, "q3", "a3", &[], true));
        assert_eq!(s.recent_context(2, 4), "user: q2\nassistant: abcd\nuser: q3\nassistant: a3");
        assert_eq!(s.recent_context(10, 100).matches("user: ").count(), 3);
    }

    #[test]
    fn a_failed_turn_keeps_a_provider_that_was_behind_owed_its_catch_up() {
        let rec = |p, ok: bool| TurnRecord { prompt: "q".into(), provider: p, model: "m".into(), reply: "r".into(), tools: vec![], ok };
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, true));
        s.record(rec(ProviderId::Claude, true));
        // codex is behind (missed 2 turns) and its attempt fails: still owed all 3
        s.record_failed(rec(ProviderId::Codex, false));
        assert_eq!(s.missed_turns(ProviderId::Codex), 3);
        // a provider that was current keeps its place when its own turn fails
        s.mark_seen(ProviderId::Claude);
        s.record_failed(rec(ProviderId::Claude, false));
        assert_eq!(s.missed_turns(ProviderId::Claude), 0);
    }

    #[test]
    fn an_undo_is_told_to_every_provider_including_the_one_that_ran_the_turn() {
        let mut s = Sessions::new();
        s.record(rec(ProviderId::Claude, "change it", "done", &["Edit a.rs"], true));
        assert_eq!(s.missed_turns(ProviderId::Claude), 0);
        s.record_undo();
        assert_eq!(s.missed_turns(ProviderId::Claude), 1);
        let text = s.catch_up(ProviderId::Claude, "", 1000).unwrap();
        assert!(text.contains("the user undid the changes of the previous turn"), "{text}");
        assert!(text.contains("FAILED"));
        assert_eq!(s.last_answered(), Some(("change it", "done")), "an undo note is not an answer");
    }
}
