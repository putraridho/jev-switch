use serde_json::json;

use super::*;

#[test]
fn provider_want_task_options_are_exactly_the_ten_task_types() {
    let q = task_type_question();
    let mut options: Vec<&str> = q["criteria"].as_object().unwrap().keys().map(String::as_str).collect();
    let mut expected: Vec<&str> = TaskType::ALL.iter().map(|t| t.as_str()).collect();
    options.sort_unstable();
    expected.sort_unstable();
    assert_eq!(options, expected);
}

#[tokio::test]
async fn the_single_call_method_sends_the_shared_question_text() {
    let (url, seen) = fake_jev(|_| (200, json!({ "answers": { "q": { "noul": 0.7 } } }).to_string())).await;
    let p = router_at(&url, 2000).security_review("can a viewer see X?", "").await.unwrap();
    assert_eq!(p, 0.7);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0]["questions"]["q"], security_review_question());
    assert_eq!(seen[0]["state"], json!({ "request": "can a viewer see X?", "recent_context": "" }));
}

#[test]
fn question_wire_format_is_unchanged() {
    let q = needs_changes_question();
    assert_eq!(q["type"], "noul");
    assert_eq!(
        q["instructions"],
        "Does fulfilling this request require modifying files or running commands in the project?"
    );
    assert!(q["criteria"]["true"].as_str().unwrap().starts_with("editing, creating"));
    assert_eq!(choice_question("i", json!({})), json!({ "type": "choice", "instructions": "i", "criteria": {} }));
}
