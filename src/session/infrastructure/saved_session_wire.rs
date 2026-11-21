//! JSON representation of saved sessions.

use serde_json::{json, Value};

use crate::session::domain::catch_up::truncate_chars;
use crate::session::domain::saved_session::{SavedSession, SAVED_REPLY_CHARS};
use crate::session::domain::session::Sessions;
use crate::session::domain::turn_record::TurnRecord;
use crate::shared::ProviderId;

/// The session as JSON for saving: CLI session ids, how far each provider has seen,
/// and the turn log with every reply cut to [`SAVED_REPLY_CHARS`].
fn sessions_to_json(sessions: &Sessions) -> Value {
    let ids: serde_json::Map<String, Value> = sessions.ids.iter().map(|(p, id)| (p.as_str().into(), json!(id))).collect();
    let seen: serde_json::Map<String, Value> = sessions.seen.iter().map(|(p, n)| (p.as_str().into(), json!(n))).collect();
    let log: Vec<Value> = sessions
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
    if let Some(name) = &sessions.name {
        out["name"] = json!(name);
    }
    out
}

/// Read what `sessions_to_json` wrote. Tolerant: a field that is missing or the
/// wrong type is skipped, and an entry that cannot be read is dropped, so a damaged
/// file gives a shorter session rather than an error.
fn sessions_from_json(v: &Value) -> Sessions {
    let provider = |s: &str| s.parse::<ProviderId>().ok();
    let mut out = Sessions::default();
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

fn to_json(saved: &SavedSession) -> Value {
    json!({
        "version": 1,
        "project": saved.project,
        "saved_ms": saved.saved_ms,
        "current": saved.current.as_ref().map(|(p, m)| json!({ "provider": p.as_str(), "model": m })),
        "sessions": sessions_to_json(&saved.sessions),
    })
}

/// `Err` only when the value is not a saved session at all (no project or time).
fn from_json(v: &Value) -> Result<SavedSession, String> {
    let project = v.get("project").and_then(Value::as_str).ok_or("no project")?.to_string();
    let saved_ms = v.get("saved_ms").and_then(Value::as_u64).ok_or("no save time")?;
    let current = v.get("current").and_then(|c| {
        let provider = c.get("provider")?.as_str()?.parse::<ProviderId>().ok()?;
        Some((provider, c.get("model")?.as_str()?.to_string()))
    });
    let sessions = sessions_from_json(v.get("sessions").unwrap_or(&Value::Null));
    Ok(SavedSession { project, saved_ms, sessions, current })
}

/// Read the text of a session file.
pub fn parse(text: &str) -> Result<SavedSession, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    from_json(&value)
}

/// Render a saved session in the on-disk format.
pub fn render(saved: &SavedSession) -> String {
    to_json(saved).to_string()
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
        let back = sessions_from_json(&sessions_to_json(&s));
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
        let back = sessions_from_json(&sessions_to_json(&s));
        assert_eq!(back.name(), Some("parser fix"));
        assert_eq!(back.title().as_deref(), Some("parser fix"));
        s.set_name(Some("   "));
        assert_eq!(s.name(), None, "a blank name takes the name away");
        assert!(sessions_to_json(&s).get("name").is_none());
    }

    #[test]
    fn from_json_survives_garbage_and_missing_fields() {
        assert_eq!(parse("{}").unwrap_err(), "no project");
        assert_eq!(parse(r#"{"project":"/work"}"#).unwrap_err(), "no save time");
        let malformed = "{ broken";
        assert_eq!(parse(malformed).unwrap_err(), serde_json::from_str::<Value>(malformed).unwrap_err().to_string());
        let legacy = parse(r#"{"project":"/work","saved_ms":1,"current":{"provider":"unknown","model":"m"}}"#).unwrap();
        assert_eq!(legacy.project, "/work");
        assert_eq!(legacy.saved_ms, 1);
        assert!(legacy.current.is_none());
        assert_eq!(legacy.sessions.turns(), 0);
        for v in [json!(null), json!(7), json!("x"), json!([]), json!({}), json!({ "log": "no", "ids": 3, "seen": [1] })] {
            let s = sessions_from_json(&v);
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
        let s = sessions_from_json(&v);
        assert_eq!(s.turns(), 2);
        assert_eq!(s.log[1].tools, ["Edit a"]);
        assert!(!s.log[1].ok && s.log[1].reply.is_empty());
        assert_eq!(s.session_id(ProviderId::Claude), None);
        assert_eq!(s.session_id(ProviderId::Codex), Some("c1"));
        assert_eq!(s.missed_turns(ProviderId::Claude), 0, "a seen count past the end is clamped");
    }
}
