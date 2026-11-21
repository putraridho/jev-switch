use serde_json::json;

use super::*;

#[tokio::test]
async fn provider_want_decodes_every_answer_from_one_request() {
    let (url, seen) = fake_jev(answer_all).await;
    let w = router_at(&url, 2000)
        .provider_want("make the sidebar collapse", "ctx", Some(("earlier ask", "earlier answer")), true)
        .await
        .expect("answered");
    assert_eq!(w.task, TaskType::Frontend);
    assert_eq!(w.task_confidence, 0.8);
    assert_eq!((w.continues_p, w.edit_p, w.security_p, w.very_hard_p), (0.11, 0.22, 0.33, 0.44));
    assert_eq!(w.correction_p, Some(0.55));
    assert_eq!(w.orchestration.multi_part_p, 0.66);
    assert_eq!(w.security_keywords, Security::None);
    assert_eq!(w.signals.to_json()["task"], "frontend");
    assert_eq!(w.signals.to_json()["security_p"], 0.33);
    // exactly one request carried all seven questions and the previous exchange
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "one HTTP request, not one per question");
    assert_eq!(seen[0]["questions"].as_object().unwrap().len(), 8);
    assert_eq!(seen[0]["state"]["previous_answer"], "earlier answer");
    assert_eq!(seen[0]["state"]["request"], "make the sidebar collapse");
}

#[tokio::test]
async fn provider_want_without_a_previous_turn_omits_the_correction_question() {
    let (url, seen) = fake_jev(answer_all).await;
    let w = router_at(&url, 2000).provider_want("rename usr to user", "", None, true).await.expect("answered");
    assert_eq!(w.correction_p, None);
    assert!(w.signals.to_json()["correction_p"].is_null());
    let seen = seen.lock().unwrap();
    let questions = seen[0]["questions"].as_object().unwrap();
    assert_eq!(questions.len(), 7);
    assert_eq!(questions["multi_part"], multi_part_question(), "the batch sends the shared definition");
    assert!(!questions.contains_key("is_correction"));
    assert!(seen[0]["state"].get("previous_request").is_none());
}


#[test]
fn local_questions_are_not_current_info() {
    for prompt in ["does jev-code has plan mode?", "does this repo support X", "is there a /foo command in jev-code"] {
        assert_eq!(repo_question_task(prompt, TaskType::CurrentInfo), TaskType::Explain, "{prompt}");
    }
    for prompt in ["what's the latest rust version", "today's news about X", "what is the latest release of jev-code"] {
        assert_eq!(repo_question_task(prompt, TaskType::CurrentInfo), TaskType::CurrentInfo, "{prompt}");
    }
}

#[tokio::test]
async fn a_stalled_jev_gives_no_provider_want_within_the_timeout() {
    let url = stalled_jev().await;
    let started = std::time::Instant::now();
    let w = router_at(&url, 200).provider_want("why does the upload hang", "", None, true).await;
    assert!(w.is_none());
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
}

#[tokio::test]
async fn a_jev_error_gives_no_provider_want() {
    let (url, _) = fake_jev(|_| (500, "boom".into())).await;
    assert!(router_at(&url, 2000).provider_want("anything", "", None, true).await.is_none());
    // a well-formed reply with an unknown task name is no answer either
    let (url, _) = fake_jev(|_| {
        (200, json!({ "answers": { "task_type": { "choice": "astrology", "confidence": 1.0 } } }).to_string())
    })
    .await;
    assert!(router_at(&url, 2000).provider_want("anything", "", None, true).await.is_none());
}
