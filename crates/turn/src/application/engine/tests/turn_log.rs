//! The rows a turn appends to the turn log.

use super::*;

#[tokio::test]
async fn successful_turns_append_log_rows_with_attempt_and_failover_notes() {
    let path = std::env::temp_dir().join(format!("jev-turn-test-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    r.engine = r.engine.with_turn_log(Some(path.to_string_lossy().into_owned()));
    let a = first_choice(TaskType::Feature, &ready);
    r.script(a, QUOTA);
    r.turn("add a retry to the client", want(TaskType::Feature)).await;
    let rows: Vec<Value> =
        std::fs::read_to_string(&path).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let _ = std::fs::remove_file(&path);
    assert_eq!(rows.len(), 2, "one row per attempt");
    assert_eq!((rows[0]["ok"].clone(), rows[0]["error_kind"].clone(), rows[0]["attempt"].clone()), (json!(false), json!("quota"), json!(1)));
    assert_eq!((rows[1]["ok"].clone(), rows[1]["attempt"].clone()), (json!(true), json!(2)));
    assert_eq!(rows[1]["signals"]["task"], "feature");
    assert!(rows[1]["reason"].as_str().unwrap().contains("task:"));
    assert!(rows[1]["failover_notes"][0].as_str().unwrap().contains("hit its limit"));
}

/// Replace the digits after `"key":` with `token`, so a per-run value cannot differ.
fn mask_number(text: &str, key: &str, token: &str) -> String {
    let pat = format!("\"{key}\":");
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(&pat) {
        let (head, tail) = rest.split_at(i + pat.len());
        out.push_str(head);
        out.push_str(token);
        rest = tail.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    out.push_str(rest);
    out
}

/// The rows one turn appended to a fresh log, `ts_ms` and `wall_ms` masked. `script` makes
/// the first-choice provider fail.
async fn logged_rows(tag: &str, script: Option<&'static str>, signals: Value) -> String {
    let path = std::env::temp_dir().join(format!("jev-golden-turnlog-{tag}-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    r.engine = r.engine.with_turn_log(Some(path.to_string_lossy().into_owned()));
    if let Some(s) = script {
        r.script(first_choice(TaskType::Feature, &ready), s);
    }
    r.turn("add a retry to the client", ProviderWant { signals: signals.into(), ..want(TaskType::Feature) }).await;
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    mask_number(&mask_number(&text, "ts_ms", "\"<TS>\""), "wall_ms", "\"<MS>\"")
}

#[tokio::test]
async fn golden_turn_log_row() {
    let typical = logged_rows("a", None, json!({ "task": "feature" })).await;
    let odd = logged_rows("b", Some(QUOTA), json!({ "task": "feature", "note": "tab\t \"quoted\" é ✓ \\ \u{1}", "none": null })).await;
    let actual = format!("## typical: one successful attempt\n{typical}## unusual: quota failover, escaped characters in signals\n{odd}");
    let golden = golden!("turn/turn_log_row.txt");
    assert_eq!(golden.split_once('\n').unwrap().1, actual);
}
