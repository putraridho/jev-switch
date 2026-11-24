use super::*;
use std::sync::Mutex;
use std::future::Future;
use std::pin::Pin;
use crate::infrastructure::claude::claude_cli::ClaudeBackend;

struct Eliciting(String);
impl Backend for Eliciting {
    fn provider(&self) -> ProviderId { ProviderId::Claude }
    fn command(&self, _: &TurnRequest) -> tokio::process::Command {
        let mut c = tokio::process::Command::new("sh"); c.args(["-c", &self.0]); c
    }
    fn stdin_payload(&self, _: &TurnRequest) -> Option<String> { Some("prompt\n".into()) }
    fn interactive(&self) -> bool { true }
    fn parse_line(&self, line: &str) -> Vec<Event> {
        if let Some(text) = line.trim_end().strip_prefix("FIN:") {
            vec![Event::Finished(Finish { text: text.into(), session_id: None, usage: Usage::default() })]
        } else { ClaudeBackend::claude().parse_line(line) }
    }
    fn elicitation_response(&self, req: &ElicitationRequest, answer: &ElicitationAnswer) -> Option<String> {
        ClaudeBackend::claude().elicitation_response(req, answer)
    }
    fn classify_error(&self, _: &str) -> ErrorKind { ErrorKind::Other }
}

const ASK: &str = r#"echo '{"type":"control_request","request_id":"e1","request":{"subtype":"elicitation","mcp_server_name":"test","message":"Confirm?","requested_schema":{"type":"object"}}}'"#;

#[derive(Default)]
struct Person { log: Mutex<Vec<String>>, wait: Duration }
impl Approver for Person {
    fn decide<'a>(&'a self, _: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> { panic!("an elicitation is not a tool approval") }
    fn elicit<'a>(&'a self, req: &'a ElicitationRequest) -> Pin<Box<dyn Future<Output = ElicitationAnswer> + Send + 'a>> {
        self.log.lock().unwrap().push(format!("asked {}", req.id));
        Box::pin(async move { tokio::time::sleep(self.wait).await; ElicitationAnswer::Accept(vec![]) })
    }
    fn elicitation_withdrawn(&self, req: &ElicitationRequest) { self.log.lock().unwrap().push(format!("withdrawn {}", req.id)); }
}

#[tokio::test]
async fn an_elicitation_waits_past_the_idle_timeout_and_replies_once() {
    let script = format!("read p; {ASK}; read reply; echo \"FIN:$reply\"");
    let person = Person { wait: Duration::from_millis(100), ..Person::default() };
    let out = run_turn_with(&Eliciting(script), &req(), Duration::from_millis(50), &mut |_| {}, &person).await.unwrap();
    let reply: serde_json::Value = serde_json::from_str(&out.text).unwrap();
    assert_eq!(reply["response"]["request_id"], "e1");
    assert_eq!(reply["response"]["response"]["action"], "accept");
    assert_eq!(*person.log.lock().unwrap(), ["asked e1"]);
}

#[tokio::test]
async fn a_run_without_a_person_returns_cancel_instead_of_hanging() {
    let script = format!("read p; {ASK}; read reply; echo \"FIN:$reply\"");
    let out = run_turn_with(&Eliciting(script), &req(), Duration::from_secs(2), &mut |_| {}, &NoApprover).await.unwrap();
    let reply: serde_json::Value = serde_json::from_str(&out.text).unwrap();
    assert_eq!(reply["response"]["response"], serde_json::json!({"action":"cancel"}));
}

#[tokio::test]
async fn withdrawal_and_result_abandon_the_question_without_writing_an_answer() {
    for ending in [r#"echo '{"type":"control_cancel_request","request_id":"e1"}'; echo 'FIN:done'"#, "echo 'FIN:done'"] {
        let script = format!("read p; {ASK}; sleep 0.1; {ending}; read late; if [ -n \"$late\" ]; then exit 9; fi");
        let person = Person { wait: Duration::from_secs(60), ..Person::default() };
        let out = run_turn_with(&Eliciting(script), &req(), Duration::from_secs(2), &mut |_| {}, &person).await.unwrap();
        assert_eq!(out.text, "done");
        assert_eq!(*person.log.lock().unwrap(), ["asked e1", "withdrawn e1"]);
    }
}
