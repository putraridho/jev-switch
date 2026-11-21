use super::*;
#[cfg(unix)]
use crate::infrastructure::gemini::gemini_cli::{Flavor, GeminiBackend};
#[cfg(unix)]
use crate::{self as backend, Approver, Backend, Decision, PermissionRequest};

#[cfg(unix)]
#[tokio::test]
async fn actual_runner_exchanges_setup_and_tool_approval_over_pipes() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;
    let dir = std::env::temp_dir().join(format!("jev-acp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("fake-agy");
    std::fs::write(&script, r##"#!/usr/bin/python3
import sys,json
assert sys.argv[1:] == ['acp']
def read(): return json.loads(sys.stdin.readline())
def send(obj): print(json.dumps(obj), flush=True)
assert read()['method'] == 'initialize'
send({'id':1,'result':{'protocolVersion':1}})
assert read()['method'] == 'session/new'
send({'id':2,'result':{'sessionId':'s','modes':{'availableModes':[{'id':'accept-edits'}]}}})
assert read()['params']['modelId'] == 'gemini-3.8-flash'
send({'id':3,'result':{}})
assert read()['params']['modeId'] == 'accept-edits'
send({'id':4,'result':{}})
assert read()['method'] == 'session/prompt'
send({'id':99,'method':'session/request_permission','params':{'sessionId':'s','toolCall':{'toolCallId':'c','kind':'execute','name':'run_command','rawInput':{'command':'git status --short'}},'options':[{'kind':'allow_once','optionId':'ok'},{'kind':'reject_once','optionId':'no'}]}})
decision = read()['result']['outcome']['optionId']
assert decision in ('ok', 'no')
send({'method':'session/update','params':{'sessionId':'s','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'Clean.' if decision == 'ok' else 'Command denied.'}}}})
send({'id':5,'result':{'stopReason':'end_turn'}})
assert sys.stdin.read() == ''
"##).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    struct Allow;
    impl Approver for Allow {
        fn decide<'a>(
            &'a self,
            req: &'a PermissionRequest,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Decision> + Send + 'a>> {
            assert_eq!(req.tool, "Bash");
            assert_eq!(req.summary, "git status --short");
            Box::pin(async { Decision::Allow })
        }
    }
    let backend = GeminiBackend::new(Flavor::Antigravity, script.to_string_lossy());
    assert!(backend.interactive());
    let outcome =
        backend::run_turn_with(&backend, &request(Mode::Auto, None), Duration::from_secs(5), &mut |_| {}, &Allow)
            .await;
    let denied = backend::run_turn_with(
        &backend,
        &request(Mode::Auto, None),
        Duration::from_secs(5),
        &mut |_| {},
        &backend::NoApprover,
    )
    .await;
    std::fs::remove_dir_all(dir).unwrap();
    assert_eq!(denied.unwrap().text, "Command denied.");
    let outcome = outcome.unwrap();
    assert_eq!(outcome.text, "Clean.");
    assert_eq!(outcome.session_id.as_deref(), Some("s"));
}
