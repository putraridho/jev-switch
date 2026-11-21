use super::*;

#[test]
fn all_modes_and_resumed_sessions_complete_setup() {
    for mode in Mode::ALL {
        for session in [None, Some("s")] {
            setup(&mut Acp::default(), mode, session);
        }
    }
}


#[test]
fn errors_cancellation_and_blank_answers_remain_failures() {
    let mut acp = Acp::default();
    setup(&mut acp, Mode::Edit, None);
    for ev in [
        json!({"id":3,"error":{"message":"unknown model"}}),
        json!({"id":5,"result":{"stopReason":"cancelled"}}),
        json!({"id":5,"result":{"stopReason":"end_turn"}}),
    ] {
        assert!(matches!(&acp.parse(&ev)[..], [Event::Failed { .. }]));
    }
}

#[test]
fn replayed_history_is_not_the_new_answer() {
    let mut acp = Acp::default();
    acp.start(&request(Mode::Edit, Some("s")));
    acp.parse(&json!({"id":1,"result":{"protocolVersion":1}}));
    assert!(acp
        .parse(&update(
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old answer"}})
        ))
        .is_empty());
    assert!(acp.text.is_empty());
}
