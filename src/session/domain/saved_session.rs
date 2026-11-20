//! A session as it is saved: the JSON it is written as, what the UI shows to offer resuming it,
//! and why resuming can fail. Reading and writing the files is `infrastructure/session_file.rs`.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::catch_up::truncate_chars;
use super::session::Sessions;
use super::turn_record::TurnRecord;
use crate::shared::ProviderId;

/// Longest reply kept in a saved session; the catch-up shrinks replies far below this anyway.
pub const SAVED_REPLY_CHARS: usize = 4_000;
impl Sessions {
    /// The session as JSON for saving: CLI session ids, how far each provider has seen,
    /// and the turn log with every reply cut to [`SAVED_REPLY_CHARS`].
    pub fn to_json(&self) -> Value {
        let ids: serde_json::Map<String, Value> = self.ids.iter().map(|(p, id)| (p.as_str().into(), json!(id))).collect();
        let seen: serde_json::Map<String, Value> = self.seen.iter().map(|(p, n)| (p.as_str().into(), json!(n))).collect();
        let log: Vec<Value> = self
            .log
            .iter()
            .map(|r| {
                json!({
                    "prompt": r.prompt,
                    "provider": r.provider.as_str(),
                    "model": r.model,
                    "reply": truncate_chars(&r.reply, SAVED_REPLY_CHARS),
                    "tools": r.tools,
                    "ok": r.ok,
                })
            })
            .collect();
        let mut out = json!({ "ids": ids, "seen": seen, "log": log });
        if let Some(name) = &self.name {
            out["name"] = json!(name);
        }
        out
    }

    /// Read what [`Sessions::to_json`] wrote. Tolerant: a field that is missing or the
    /// wrong type is skipped, and an entry that cannot be read is dropped, so a damaged
    /// file gives a shorter session rather than an error.
    pub fn from_json(v: &Value) -> Self {
        let provider = |s: &str| s.parse::<ProviderId>().ok();
        let mut out = Self::default();
        out.set_name(v.get("name").and_then(Value::as_str));
        if let Some(entries) = v.get("log").and_then(Value::as_array) {
            for e in entries {
                let text = |k: &str| e.get(k).and_then(Value::as_str);
                let (Some(prompt), Some(p)) = (text("prompt"), text("provider").and_then(provider)) else { continue };
                out.log.push(TurnRecord {
                    prompt: prompt.to_string(),
                    provider: p,
                    model: text("model").unwrap_or_default().to_string(),
                    reply: text("reply").unwrap_or_default().to_string(),
                    tools: e
                        .get("tools")
                        .and_then(Value::as_array)
                        .map(|t| t.iter().filter_map(Value::as_str).map(String::from).collect())
                        .unwrap_or_default(),
                    ok: e.get("ok").and_then(Value::as_bool).unwrap_or(false),
                });
            }
        }
        if let Some(ids) = v.get("ids").and_then(Value::as_object) {
            for (k, id) in ids {
                if let (Some(p), Some(id)) = (provider(k), id.as_str().filter(|s| !s.is_empty())) {
                    out.ids.insert(p, id.to_string());
                }
            }
        }
        if let Some(seen) = v.get("seen").and_then(Value::as_object) {
            for (k, n) in seen {
                if let (Some(p), Some(n)) = (provider(k), n.as_u64()) {
                    out.seen.insert(p, (n as usize).min(out.log.len()));
                }
            }
        }
        out
    }
}

/// A session saved to disk is ignored once it is older than this.
pub const SAVED_MAX_AGE_MS: u64 = 14 * 24 * 60 * 60 * 1000;

/// What is stored for one project: the conversation plus where routing stood.
#[derive(Debug, Clone)]
pub struct SavedSession {
    pub project: String,
    pub saved_ms: u64,
    pub sessions: Sessions,
    /// The model the last turn ran on.
    pub current: Option<(ProviderId, String)>,
}

/// What the UI shows to offer resuming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub turns: usize,
    pub last_used_ms: u64,
    pub providers: Vec<ProviderId>,
    /// The model the last turn ran on.
    pub current: Option<(ProviderId, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeError {
    /// Persistence is switched off for this engine.
    Disabled,
    /// Nothing saved for this project (or it is older than 14 days, or for another project).
    NotFound,
    /// The file is not a saved session.
    Corrupt(String),
    Io(String),
}

impl std::fmt::Display for ResumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("saving sessions is switched off"),
            Self::NotFound => f.write_str("no earlier saved session for this folder (the last 10 are kept, for 14 days)"),
            Self::Corrupt(m) => write!(f, "the saved session is unreadable: {m}"),
            Self::Io(m) => write!(f, "could not read the saved session: {m}"),
        }
    }
}

impl std::error::Error for ResumeError {}

impl SavedSession {
    pub fn to_json(&self) -> Value {
        json!({
            "version": 1,
            "project": self.project,
            "saved_ms": self.saved_ms,
            "current": self.current.as_ref().map(|(p, m)| json!({ "provider": p.as_str(), "model": m })),
            "sessions": self.sessions.to_json(),
        })
    }

    /// `Err` only when the value is not a saved session at all (no project or time).
    pub fn from_json(v: &Value) -> Result<Self, String> {
        let project = v.get("project").and_then(Value::as_str).ok_or("no project")?.to_string();
        let saved_ms = v.get("saved_ms").and_then(Value::as_u64).ok_or("no save time")?;
        let current = v.get("current").and_then(|c| {
            let provider = c.get("provider")?.as_str()?.parse::<ProviderId>().ok()?;
            Some((provider, c.get("model")?.as_str()?.to_string()))
        });
        let sessions = Sessions::from_json(v.get("sessions").unwrap_or(&Value::Null));
        Ok(Self { project, saved_ms, sessions, current })
    }

    /// Read the text of a session file.
    pub fn parse(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        Self::from_json(&value)
    }

    /// The text of a session file, as [`SavedSession::parse`] reads it.
    pub fn render(&self) -> String {
        self.to_json().to_string()
    }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            turns: self.sessions.turns(),
            last_used_ms: self.saved_ms,
            providers: self.sessions.providers_used(),
            current: self.current.clone(),
        }
    }

    /// Is this too old to offer at time `now_ms`?
    pub fn expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.saved_ms) > SAVED_MAX_AGE_MS
    }
}

/// One saved session as the `/resume` list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    pub path: PathBuf,
    pub info: SessionInfo,
    /// What the person first asked.
    pub title: String,
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
    fn sessions_round_trip_through_json_and_cap_long_replies() {
        let mut s = Sessions::new();
        s.set_session_id(ProviderId::Claude, "abc".into());
        s.set_session_id(ProviderId::Codex, "xyz".into());
        s.record(rec(ProviderId::Claude, "one", "r1", &["Edit src/a.rs"], true));
        s.record(rec(ProviderId::Codex, "two", &"r".repeat(SAVED_REPLY_CHARS + 500), &[], false));
        let back = Sessions::from_json(&s.to_json());
        assert_eq!(back.session_id(ProviderId::Claude), Some("abc"));
        assert_eq!(back.session_id(ProviderId::Codex), Some("xyz"));
        assert_eq!(back.turns(), 2);
        assert_eq!(back.missed_turns(ProviderId::Codex), 0);
        assert_eq!(back.missed_turns(ProviderId::Claude), 1, "who has seen what survives");
        assert_eq!(back.log[0], s.log[0]);
        assert!(!back.log[1].ok);
        assert_eq!(back.log[1].reply.chars().count(), SAVED_REPLY_CHARS + 1, "capped, with an ellipsis");
        assert_eq!(back.providers_used(), [ProviderId::Claude, ProviderId::Codex]);
        assert_eq!(back.last_answered(), Some(("one", "r1")));
    }

    #[test]
    fn a_name_is_saved_and_read_back_and_titles_the_session() {
        let mut s = Sessions::new();
        assert_eq!(s.title(), None);
        s.set_name(Some("  parser   fix "));
        assert_eq!(s.name(), Some("parser fix"));
        let back = Sessions::from_json(&s.to_json());
        assert_eq!(back.name(), Some("parser fix"));
        assert_eq!(back.title().as_deref(), Some("parser fix"));
        s.set_name(Some("   "));
        assert_eq!(s.name(), None, "a blank name takes the name away");
        assert!(s.to_json().get("name").is_none());
    }

    #[test]
    fn from_json_survives_garbage_and_missing_fields() {
        for v in [json!(null), json!(7), json!("x"), json!([]), json!({}), json!({ "log": "no", "ids": 3, "seen": [1] })] {
            let s = Sessions::from_json(&v);
            assert_eq!((s.turns(), s.session_id(ProviderId::Claude)), (0, None), "{v}");
        }
        let v = json!({
            "log": [
                { "prompt": "ok", "provider": "claude" },
                { "prompt": "unknown provider", "provider": "nope" },
                { "provider": "claude" },
                7,
                { "prompt": "tools are junk", "provider": "codex", "tools": [1, "Edit a", null], "ok": "yes", "reply": 5 },
            ],
            "ids": { "claude": "", "codex": "c1", "nope": "x", "grok": 5 },
            "seen": { "claude": 99, "codex": "many", "nope": 1 },
        });
        let s = Sessions::from_json(&v);
        assert_eq!(s.turns(), 2);
        assert_eq!(s.log[1].tools, ["Edit a"]);
        assert!(!s.log[1].ok && s.log[1].reply.is_empty());
        assert_eq!(s.session_id(ProviderId::Claude), None);
        assert_eq!(s.session_id(ProviderId::Codex), Some("c1"));
        assert_eq!(s.missed_turns(ProviderId::Claude), 0, "a seen count past the end is clamped");
    }
}
