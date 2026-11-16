//! One JSON row per coding turn, appended with [`crate::log::append`], so cost,
//! failures and quota hits per provider can be analysed later with jq or DuckDB.

use serde_json::{json, Value};

use crate::backend::{ErrorKind, PlanUsage, ProviderId, TurnError, TurnOutcome, Usage};
use crate::log::now_ms;

fn error_kind_name(kind: &ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Quota { .. } => "quota",
        ErrorKind::NotEntitled => "not_entitled",
        ErrorKind::Auth => "auth",
        ErrorKind::Empty => "empty",
        ErrorKind::Stalled => "stalled",
        ErrorKind::Other => "other",
    }
}

fn usage_json(u: &Usage) -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;

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
