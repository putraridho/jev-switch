use std::time::Instant;

use axum::http::StatusCode;
use serde_json::{json, Value};

use jev_catalog::domain::model_table::{MODEL_CHEAP, MODEL_MID};
use crate::domain::auxiliary_cooldown::AUXILIARY_COOLDOWN;

use super::golden_rig::{anthropic_answers, jev_answers, Rig};

const REQUEST: &str = r#"{"model":"jev-code","max_tokens":5,"thinking":{"type":"adaptive"},"messages":[{"role":"user","content":"title this"}]}"#;

fn rejected(body: &str) -> (u16, &'static str, String) {
    let request: Value = serde_json::from_str(body).unwrap();
    (429, "application/json", json!({"type":"error","error":{"type":"rate_limit_error","message":request["model"]}}).to_string())
}

fn models(rig: &Rig) -> Vec<String> {
    rig.upstream.lock().unwrap().iter().map(|body| serde_json::from_str::<Value>(body).unwrap()["model"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn repeated_auxiliary_429s_stop_at_both_models_across_sessions_and_turns() {
    let rig = Rig::with_upstream("auxiliary-429", jev_answers, rejected).await;
    let (status, _, body) = rig.send_response("first", Some("turn-1"), "auxiliary", REQUEST).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["message"], MODEL_MID);
    for index in 0..4 {
        let (status, headers, body) = rig.send_response(&format!("session-{index}"), Some(&format!("turn-{index}")), "auxiliary", REQUEST).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!((1..=60).contains(&headers["retry-after"].to_str().unwrap().parse::<u64>().unwrap()));
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["type"], "rate_limit_error");
    }
    assert_eq!(models(&rig), [MODEL_CHEAP, MODEL_MID]);
    let usage: Value = serde_json::from_str(rig.log("usage.jsonl").trim()).unwrap();
    assert_eq!(usage["model"], MODEL_MID);
    assert_eq!(usage["fallback_from"]["status"], 429);
}

#[tokio::test]
async fn haiku_429_uses_sonnet_without_adapting_later_auxiliary_requests() {
    fn reply(body: &str) -> (u16, &'static str, String) {
        if serde_json::from_str::<Value>(body).unwrap()["model"] == MODEL_CHEAP {
            rejected(body)
        } else {
            anthropic_answers(body)
        }
    }
    let rig = Rig::with_upstream("auxiliary-sonnet", jev_answers, reply).await;
    for session in ["first", "second"] {
        assert_eq!(rig.send_response(session, None, "auxiliary", REQUEST).await.0, StatusCode::OK);
    }
    assert_eq!(models(&rig), [MODEL_CHEAP, MODEL_MID, MODEL_MID]);
    let bodies = rig.upstream.lock().unwrap();
    assert!(serde_json::from_str::<Value>(&bodies[0]).unwrap().get("thinking").is_none());
    for body in &bodies[1..] {
        let actual: Value = serde_json::from_str(body).unwrap();
        let mut expected: Value = serde_json::from_str(REQUEST).unwrap();
        expected["model"] = json!(MODEL_MID);
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn a_sonnet_first_auxiliary_429_is_not_retried_on_sonnet() {
    let rig = Rig::with_upstream("auxiliary-sonnet-first", jev_answers, rejected).await;
    rig.app.auxiliary_cooldown.lock().await.reject_rate_limited(MODEL_CHEAP, None, Instant::now());
    let (status, _, body) = rig.send_response("first", None, "auxiliary", REQUEST).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["message"], MODEL_MID);
    rig.send("second", None, "auxiliary", REQUEST).await;
    assert_eq!(models(&rig), [MODEL_MID]);
}

#[tokio::test]
async fn auxiliary_cooldowns_do_not_change_main_forks_subagents_or_explicit_models() {
    let rig = Rig::with_upstream("auxiliary-scope", jev_answers, rejected).await;
    rig.send("first", None, "auxiliary", REQUEST).await;
    let continuation = r#"{"model":"jev-code","max_tokens":5,"messages":[]}"#;
    rig.send("main", None, "main", continuation).await;
    rig.send("subagent", None, "subagent", continuation).await;
    let mut fork: Value = serde_json::from_str(REQUEST).unwrap();
    fork["tools"] = json!([{"name":"Read","input_schema":{"type":"object"}}]);
    rig.send("fork", None, "auxiliary", &fork.to_string()).await;
    let mut explicit: Value = serde_json::from_str(REQUEST).unwrap();
    explicit["model"] = json!(MODEL_CHEAP);
    rig.send("explicit", None, "auxiliary", &explicit.to_string()).await;
    assert_eq!(models(&rig), [MODEL_CHEAP, MODEL_MID, MODEL_MID, MODEL_MID, MODEL_MID, MODEL_CHEAP]);
}

#[tokio::test]
async fn an_expired_haiku_cooldown_does_not_retry_a_still_cooling_sonnet() {
    let rig = Rig::with_upstream("auxiliary-expiry", jev_answers, rejected).await;
    let now = Instant::now();
    {
        let mut cooldown = rig.app.auxiliary_cooldown.lock().await;
        cooldown.reject_rate_limited(MODEL_CHEAP, None, now - AUXILIARY_COOLDOWN);
        cooldown.reject_rate_limited(MODEL_MID, None, now);
    }
    let (status, _, body) = rig.send_response("recovered", None, "auxiliary", REQUEST).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["message"], MODEL_CHEAP);
    assert_eq!(models(&rig), [MODEL_CHEAP]);
}

#[tokio::test]
async fn expired_auxiliary_cooldowns_allow_haiku_to_recover() {
    let rig = Rig::new("auxiliary-recovery", jev_answers).await;
    {
        let mut cooldown = rig.app.auxiliary_cooldown.lock().await;
        let expired = Instant::now() - AUXILIARY_COOLDOWN;
        for model in [MODEL_CHEAP, MODEL_MID] {
            cooldown.reject_rate_limited(model, None, expired);
        }
    }
    assert_eq!(rig.send_response("recovered", None, "auxiliary", REQUEST).await.0, StatusCode::OK);
    assert_eq!(models(&rig), [MODEL_CHEAP]);
}

#[tokio::test]
async fn other_rejections_keep_existing_per_request_fallback_without_cooldowns() {
    fn reply(body: &str) -> (u16, &'static str, String) {
        (400, "application/json", body.to_string())
    }
    let rig = Rig::with_upstream("auxiliary-400", jev_answers, reply).await;
    for session in ["first", "second"] {
        assert_eq!(rig.send_response(session, None, "auxiliary", REQUEST).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(models(&rig), [MODEL_CHEAP, MODEL_MID, MODEL_CHEAP, MODEL_MID]);
}

#[tokio::test]
async fn concurrent_auxiliary_rejections_share_one_bounded_fallback() {
    let rig = Rig::with_upstream("auxiliary-concurrent", jev_answers, rejected).await;
    let requests = (0..8).map(|index| {
        let rig = &rig;
        async move { rig.send_response(&format!("session-{index}"), None, "auxiliary", REQUEST).await }
    });
    let responses = tokio::time::timeout(std::time::Duration::from_secs(5), futures_util::future::join_all(requests)).await.unwrap();
    assert!(responses.iter().all(|(status, _, _)| *status == StatusCode::TOO_MANY_REQUESTS));
    assert_eq!(responses.iter().filter(|(_, headers, _)| headers.contains_key("retry-after")).count(), 7);
    assert_eq!(models(&rig), [MODEL_CHEAP, MODEL_MID]);
}
