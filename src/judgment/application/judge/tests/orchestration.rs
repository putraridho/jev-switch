use serde_json::{json, Value};

use super::*;

#[tokio::test]
async fn with_orchestration_off_the_sequence_veto_is_not_asked() {
    let (url, seen) = fake_jev(answer_all).await;
    let w = router_at(&url, 2000).provider_want("rename usr to user", "", None, false).await.expect("answered");
    assert_eq!((w.orchestration.multi_part_p, w.orchestration.sequential_p, w.orchestration.vetoed), (0.66, None, false));
    assert!(w.signals.to_json()["sequential_p"].is_null() && w.signals.to_json().get("sequential_error").is_none());
    let seen = seen.lock().unwrap();
    let questions = seen[0]["questions"].as_object().unwrap();
    assert_eq!(questions.len(), 6);
    assert!(!questions.contains_key("sequential"), "asked only when orchestration is on");
}

/// Every answer readable except `sequential`, which has no probability.
fn unreadable_sequential(req: &Value) -> (u16, String) {
    let (_, body) = answer_all(req);
    let mut body: Value = serde_json::from_str(&body).unwrap();
    if body["answers"].get("sequential").is_some() {
        body["answers"]["sequential"] = json!({ "noul": "not a number" });
    }
    (200, body.to_string())
}

#[tokio::test]
async fn an_unreadable_sequential_answer_loses_only_the_veto() {
    let (url, _) = fake_jev(unreadable_sequential).await;
    let router = router_at(&url, 2000);
    let w = router.provider_want("make the sidebar collapse", "", None, true).await.expect("the other signals still route");
    assert_eq!(w.task, TaskType::Frontend);
    assert_eq!((w.continues_p, w.edit_p, w.security_p, w.very_hard_p), (0.11, 0.22, 0.33, 0.44));
    assert_eq!((w.orchestration.multi_part_p, w.orchestration.sequential_p, w.orchestration.vetoed), (0.66, None, false));
    assert!(w.signals.to_json()["sequential_error"].as_str().is_some_and(|e| !e.is_empty()), "the log says why: {}", w.signals.to_json());
    // The gateway's check degrades the same way.
    let j = router.orchestration_p("make the sidebar collapse", "").await.expect("multi_part was readable");
    assert_eq!((j.multi_part_p, j.sequential_p, j.vetoed), (0.66, None, false));
}

/// A multi-part request that is also a sequence: the veto holds it back.
fn multi_part_but_sequential(req: &Value) -> (u16, String) {
    let mut answers = serde_json::Map::new();
    for name in req["questions"].as_object().unwrap().keys() {
        let p = match name.as_str() {
            "multi_part" => 0.95,
            "sequential" => 0.9,
            _ => 0.1,
        };
        answers.insert(name.clone(), json!({ "noul": p }));
    }
    (200, json!({ "answers": answers }).to_string())
}

#[tokio::test]
async fn orchestration_p_keeps_the_raw_judgment_and_the_veto() {
    let (url, seen) = fake_jev(multi_part_but_sequential).await;
    let j = router_at(&url, 2000).orchestration_p("do 1, then 2, then 3", "").await.expect("answered");
    assert_eq!((j.multi_part_p, j.sequential_p, j.vetoed), (0.95, Some(0.9), true));
    assert_eq!(seen.lock().unwrap().len(), 1, "one request carries both questions");
}
