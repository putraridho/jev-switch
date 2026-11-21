//! Golden: what the gateway records and forwards, driven through `forward`.

use axum::extract::{Json, State};

use super::check_handlers::{orchestrate_check, OrchCheck};
use super::golden_rig::{jev_answers, jev_down, masked_line, Rig, GOLDEN_MASK};

/// What one scenario left behind.
struct Ran {
    name: &'static str,
    decisions: String,
    usage: String,
    turns: String,
    upstream: String,
}

const USER_ONE: &str = r#""messages":[{"role":"user","content":"fix the login bug in the settings page"}]"#;

/// Four runs: a bare request on the byte-swap path, a full session on the JSON round-trip
/// path (checked first by the orchestrate hook, with a working directory, streamed, two
/// turns), an orchestrated turn, and a small Haiku request.
async fn scenarios() -> Vec<Ran> {
    let mut ran = Vec::new();
    let finish = |name, rig: &Rig| Ran {
        name,
        decisions: rig.log("gateway.jsonl"),
        usage: rig.log("usage.jsonl"),
        turns: rig.log("turns.jsonl"),
        upstream: rig.bodies(),
    };

    // `"model":"jev-code"` exactly as set_model looks for it: only that value changes.
    let rig = Rig::new("bare", jev_answers).await;
    rig.send("s-bare", Some("p1"), "main", &format!(r#"{{"model":"jev-code","max_tokens":  10,{USER_ONE}}}"#)).await;
    ran.push(finish("bare", &rig));

    // Spaced JSON: set_model falls back to a round trip through `Value`.
    let rig = Rig::new("full", jev_answers).await;
    let check = OrchCheck { prompt: "add a login page, a settings page and tests for both".into(), context: "earlier answer".into(), session: "s-full".into() };
    let _ = orchestrate_check(State(rig.app.clone()), Json(check)).await;
    let system = r#""system":[{"type":"text","text":"Primary working directory: /work/proj\nrules"}]"#;
    rig.send("s-full", Some("p1"), "main", &format!(r#"{{ "model": "jev-code", "zeta": 1e2, "stream": true, "max_tokens": 10, {system}, {USER_ONE} }}"#)).await;
    let second = r#""messages": [{"role":"user","content":"fix the login bug"},{"role":"assistant","content":[{"type":"text","text":"Done."}]},{"role":"user","content":"now make the button blue"}]"#;
    rig.send("s-full", Some("p2"), "main", &format!(r#"{{ "model": "jev-code", "stream": true, {system}, {second} }}"#)).await;
    ran.push(finish("full", &rig));

    let rig = Rig::new("orchestrated", jev_answers).await;
    let brief = r#""messages":[{"role":"user","content":"build the feature <jev-orchestrate>plan, delegate, review</jev-orchestrate>"}]"#;
    rig.send("s-orch", Some("p1"), "main", &format!(r#"{{"model":"jev-code",{brief}}}"#)).await;
    ran.push(finish("orchestrated", &rig));

    // A small auxiliary request goes to Haiku, adapted: always re-serialised.
    let rig = Rig::new("haiku", jev_answers).await;
    let adaptive = r#""thinking":{"type":"adaptive"},"context_management":{"edits":[]},"output_config":{"effort":"high"},"messages":[{"role":"user","content":"title this"},{"role":"system","content":"note"}]"#;
    rig.send("s-haiku", None, "auxiliary", &format!(r#"{{"model":"jev-code","max_tokens":5,{adaptive}}}"#)).await;
    ran.push(finish("haiku", &rig));
    ran
}

/// Writes nothing; compares `actual` with the golden file, whose first line says what is masked.
fn compare_golden(golden: &str, actual: &str) {
    let (head, body) = golden.split_once('\n').unwrap();
    assert_eq!(head, format!("# masked: {GOLDEN_MASK}"));
    assert_eq!(body, actual);
}

fn sections(ran: &[Ran], pick: impl Fn(&Ran) -> &str) -> String {
    ran.iter().filter(|r| !pick(r).is_empty()).map(|r| format!("=== {}\n{}", r.name, pick(r))).collect()
}

#[tokio::test]
async fn golden_gateway_decision_record() {
    const GOLDEN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/golden/gateway/decision_record.txt")); // golden!
    let actual = sections(&scenarios().await, |r| &r.decisions);
    compare_golden(GOLDEN, &actual);
}

#[tokio::test]
async fn golden_gateway_usage_request() {
    const GOLDEN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/golden/gateway/usage_request.txt")); // golden!
    let actual = sections(&scenarios().await, |r| &r.usage);
    compare_golden(GOLDEN, &actual);
}

#[tokio::test]
async fn golden_gateway_turn_record() {
    const GOLDEN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/golden/gateway/turn_record.txt")); // golden!
    let actual = sections(&scenarios().await, |r| &r.turns);
    compare_golden(GOLDEN, &actual);
}

/// The request bodies the fake Anthropic received: the byte-swap path (`bare`,
/// `orchestrated`), the round-trip path (`full`) and Haiku's adapted body.
#[tokio::test]
async fn golden_gateway_upstream_body() {
    const GOLDEN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/golden/gateway/upstream_body.txt")); // golden!
    let actual = sections(&scenarios().await, |r| &r.upstream);
    compare_golden(GOLDEN, &actual);
}

/// The orchestrate check: its log line and its response, with Jev answering and with Jev down.
#[tokio::test]
async fn golden_gateway_check_record() {
    const GOLDEN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/golden/gateway/check_record.txt")); // golden!
    let mut actual = String::new();
    for (name, jev) in [("jev_answers", jev_answers as fn(&str) -> (u16, &'static str, String)), ("jev_down", jev_down)] {
        let rig = Rig::new(name, jev).await;
        let check = OrchCheck { prompt: "add a login page, a settings page and tests for both".into(), context: "earlier answer".into(), session: "s-check".into() };
        let Json(response) = orchestrate_check(State(rig.app.clone()), Json(check)).await;
        actual.push_str(&format!("=== {name}\n{}{}\n", rig.log("orch.jsonl"), masked_line(&response.to_string())));
    }
    compare_golden(GOLDEN, &actual);
}
