//! One JSON row per coding turn, appended with [`crate::log::append`], so cost,
//! failures and quota hits per provider can be analysed later with jq or DuckDB.

use serde_json::{json, Value};

use crate::backend::{ErrorKind, PlanUsage, ProviderId, TurnError, TurnOutcome, Usage};
use crate::log::now_ms;

pub fn error_kind_name(kind: &ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Quota { .. } => "quota",
        ErrorKind::NotEntitled => "not_entitled",
        ErrorKind::Auth => "auth",
        ErrorKind::Empty => "empty",
        ErrorKind::Stalled => "stalled",
        ErrorKind::UnusableHere => "unusable_here",
        ErrorKind::Other => "other",
    }
}

pub fn usage_json(u: &Usage) -> Value {
    json!({
        "input_tokens": u.input_tokens,
        "cached_input_tokens": u.cached_input_tokens,
        "output_tokens": u.output_tokens,
        "context_tokens": u.context_tokens,
    })
}

fn plan_json(p: &PlanUsage) -> Value {
    json!({ "window": p.window, "utilization": p.utilization, "resets_at": p.resets_at })
}

/// Build the row for one finished turn. Failed turns get the same keys as
/// successful ones (nulls where unknown) so a log reads as one table.
/// `session` is the session id the turn ran in; `reason` and `signals` are the
/// router's verdict for the turn.
pub fn turn_row(
    provider: ProviderId,
    model: &str,
    session: Option<&str>,
    reason: &str,
    signals: &Value,
    result: &Result<TurnOutcome, TurnError>,
    wall_ms: u64,
) -> Value {
    let (ok, usage, plan, error_kind, error) = match result {
        Ok(o) => (true, Some(usage_json(&o.usage)), o.plan.as_ref().map(plan_json), None, None),
        Err(e) => (false, None, None, Some(error_kind_name(&e.kind)), Some(e.message.as_str())),
    };
    json!({
        "ts_ms": now_ms(),
        "provider": provider.as_str(),
        "model": model,
        "session": session,
        "reason": reason,
        "signals": signals,
        "ok": ok,
        "error_kind": error_kind,
        "error": error,
        "wall_ms": wall_ms,
        "usage": usage,
        "plan": plan,
    })
}

/// The single JSON line the `ask` binary prints for one headless turn. Same shape
/// for success and failure (nulls where unknown) so a caller parses one schema.
pub fn ask_json(provider: &str, model: &str, result: &Result<TurnOutcome, TurnError>, wall_ms: u64) -> Value {
    let (ok, text, session, usage, error_kind, error) = match result {
        Ok(o) => (true, o.text.as_str(), o.session_id.as_deref(), Some(usage_json(&o.usage)), None, None),
        Err(e) => (false, "", None, None, Some(error_kind_name(&e.kind)), Some(e.message.as_str())),
    };
    json!({
        "provider": provider,
        "model": model,
        "ok": ok,
        "text": text,
        "session_id": session,
        "usage": usage,
        "error_kind": error_kind,
        "error": error,
        "wall_ms": wall_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_json_success_and_failure_share_one_schema() {
        let out = TurnOutcome {
            text: "hi".into(),
            session_id: Some("s1".into()),
            usage: Usage { input_tokens: 3, cached_input_tokens: 1, output_tokens: 2, context_tokens: None },
            plan: None,
        };
        let ok = ask_json("codex", "m", &Ok(out), 42);
        assert_eq!(ok["provider"], "codex");
        assert_eq!(ok["model"], "m");
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["text"], "hi");
        assert_eq!(ok["session_id"], "s1");
        assert_eq!(ok["usage"]["output_tokens"], 2);
        assert!(ok["error_kind"].is_null() && ok["error"].is_null());
        let err = TurnError { kind: ErrorKind::Quota { resets_at: None }, message: "limit".into() };
        let bad = ask_json("codex", "m", &Err(err), 7);
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["text"], "");
        assert_eq!(bad["error_kind"], "quota");
        assert_eq!(bad["error"], "limit");
        assert!(bad["usage"].is_null() && bad["session_id"].is_null());
        assert_eq!(bad["wall_ms"], 7);
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&ok), keys(&bad));
    }

    #[test]
    fn success_row_has_usage_and_plan() {
        let out = TurnOutcome {
            text: "done".into(),
            session_id: Some("s".into()),
            usage: Usage { input_tokens: 10, cached_input_tokens: 4, output_tokens: 2, context_tokens: Some(9) },
            plan: Some(PlanUsage { window: "five_hour".into(), utilization: 0.5, resets_at: Some(7) }),
        };
        let row = turn_row(ProviderId::Claude, "m", Some("s"), "why", &json!({"a":1}), &Ok(out), 1234);
        assert_eq!(row["provider"], "claude");
        assert_eq!(row["model"], "m");
        assert_eq!(row["session"], "s");
        assert_eq!(row["reason"], "why");
        assert_eq!(row["signals"]["a"], 1);
        assert_eq!(row["ok"], true);
        assert!(row["error_kind"].is_null() && row["error"].is_null());
        assert_eq!(row["wall_ms"], 1234);
        assert_eq!(row["usage"]["cached_input_tokens"], 4);
        assert_eq!(row["usage"]["context_tokens"], 9);
        assert_eq!(row["plan"]["window"], "five_hour");
        assert!(row["ts_ms"].as_u64().unwrap() > 0);
    }

    #[test]
    fn failure_row_has_error_and_null_usage() {
        let err = TurnError { kind: ErrorKind::Quota { resets_at: Some("5pm".into()) }, message: "limit".into() };
        let row = turn_row(ProviderId::DeepSeek, "m", None, "r", &Value::Null, &Err(err), 5);
        assert_eq!(row["provider"], "deepseek");
        assert_eq!(row["ok"], false);
        assert_eq!(row["error_kind"], "quota");
        assert_eq!(row["error"], "limit");
        assert!(row["usage"].is_null() && row["plan"].is_null() && row["session"].is_null());
    }
}
