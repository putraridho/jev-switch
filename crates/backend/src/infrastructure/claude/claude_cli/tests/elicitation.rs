use super::*;
use crate::{ElicitationRequest, ElicitationKind, ElicitationAnswer, ElicitationValue, FieldKind};

fn question(mode: Option<&str>, schema: Value) -> ElicitationRequest {
    let mut r = json!({"subtype":"elicitation","mcp_server_name":"lunagraph","message":"Run pnpm install?","requested_schema":schema});
    if let Some(mode) = mode { r["mode"] = json!(mode); }
    let events = session().parse_line(&json!({"type":"control_request","request_id":"e-1","request":r}).to_string());
    match events.into_iter().next().unwrap() { Event::ElicitationRequest(r) => r, e => panic!("{e:?}") }
}

#[test]
fn form_requests_default_to_form_and_preserve_boolean_defaults() {
    let req = question(None, json!({"type":"object","properties":{"approve":{"type":"boolean","title":"Approve","default":false}},"required":["approve"]}));
    assert_eq!(req.server, "lunagraph");
    let ElicitationKind::Form(fields) = &req.kind else { panic!("{req:?}") };
    assert_eq!(fields[0].kind, FieldKind::Boolean);
    assert!(fields[0].required);
    assert_eq!(fields[0].default, "false");
    let answer = ElicitationAnswer::Accept(vec![("approve".into(), ElicitationValue::Boolean(true))]);
    let response: Value = serde_json::from_str(&session().elicitation_response(&req, &answer).unwrap()).unwrap();
    assert_eq!(response, json!({"type":"control_response","response":{"subtype":"success","request_id":"e-1","response":{"action":"accept","content":{"approve":true}}}}));
    for (answer, action) in [(ElicitationAnswer::Decline, "decline"), (ElicitationAnswer::Cancel, "cancel")] {
        let response: Value = serde_json::from_str(&session().elicitation_response(&req, &answer).unwrap()).unwrap();
        assert_eq!(response["response"]["response"], json!({"action":action}));
    }
}

#[test]
fn field_order_bounds_and_scalar_types_are_retained() {
    let event = json!({"type":"control_request","request_id":"e-2","request":{
        "subtype":"elicitation","mcp_server_name":"forms","message":"Setup","property_order":["name","count","color","ratio"],
        "requested_schema":{"type":"object","properties":{
            "name":{"type":"string","minLength":2,"maxLength":5},
            "count":{"type":"integer","minimum":1,"maximum":5},
            "color":{"type":"string","enum":["red","blue"]},
            "ratio":{"type":"number"}
        },"required":["name","count"]}
    }});
    let Event::ElicitationRequest(req) = session().parse_line(&event.to_string()).remove(0) else { panic!() };
    let ElicitationKind::Form(fields) = req.kind else { panic!() };
    assert_eq!(fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["name","count","color","ratio"]);
    assert!(fields[0].parse("x").is_err());
    assert!(fields[0].parse("abcdef").is_err());
    assert!(fields[0].parse("").is_err());
    assert!(fields[1].parse("0").is_err());
    assert!(fields[1].parse("1.5").is_err());
    assert_eq!(fields[1].parse("3"), Ok(Some(ElicitationValue::Integer(3))));
    assert!(fields[2].parse("green").is_err());
    assert_eq!(fields[2].parse(""), Ok(None));
    assert!(fields[3].parse("NaN").is_err());
    assert_eq!(fields[3].parse("1.5"), Ok(Some(ElicitationValue::Number(1.5))));
}

#[test]
fn unknown_or_invalid_forms_still_become_answerable_requests() {
    for schema in [json!({"type":"array"}), json!({"type":"object","properties":{"nested":{"type":"object"}}}), json!({"type":"object","required":["missing"]})] {
        let req = question(Some("form"), schema);
        assert!(matches!(req.kind, ElicitationKind::Unsupported(_)));
    }
    assert!(matches!(question(Some("future"), json!({})).kind, ElicitationKind::Unsupported(_)));
    assert!(matches!(question(None, json!({"type":"object"})).kind, ElicitationKind::Form(f) if f.is_empty()));
}

#[test]
fn url_requests_are_forwarded_without_opening_a_browser() {
    let event = json!({"type":"control_request","request_id":"url-1","request":{"subtype":"elicitation","mcp_server_name":"auth","message":"Sign in","mode":"url","url":"https://example.test/auth","elicitation_id":"server-id"}});
    let Event::ElicitationRequest(req) = session().parse_line(&event.to_string()).remove(0) else { panic!() };
    assert!(matches!(&req.kind, ElicitationKind::Url(url) if url == "https://example.test/auth"));
    let response: Value = serde_json::from_str(&session().elicitation_response(&req, &ElicitationAnswer::Accept(vec![])).unwrap()).unwrap();
    assert_eq!(response["response"]["request_id"], "url-1");
    assert_eq!(response["response"]["response"], json!({"action":"accept"}));
}

#[test]
fn debug_does_not_disclose_questions_or_answers() {
    let req = question(None, json!({"type":"object"}));
    assert!(!format!("{req:?}").contains("pnpm"));
    assert!(!format!("{:?}", ElicitationAnswer::Accept(vec![("token".into(), ElicitationValue::Text("SECRET".into()))])).contains("SECRET"));
}
