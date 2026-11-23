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
fn occupancy_updates_do_not_report_consumed_usage_and_each_turn_resets_it() {
    for agent in [Agent::Antigravity, Agent::Grok] {
        for context_tokens in [None, Some(0), Some(50_000)] {
            let mut acp = Acp::for_agent(agent);
            setup(&mut acp, Mode::Ask, None);
            if let Some(used) = context_tokens {
                acp.parse(&update(json!({"sessionUpdate":"usage_update","used":used})));
            }
            acp.parse(&update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"done"}})));
            let events = acp.parse(&json!({"id":5,"result":{"stopReason":"end_turn"}}));
            let Some(Event::Finished(finish)) = events.last() else { panic!("{events:?}") };
            assert_eq!(finish.usage, Usage { measurement: Measurement::Unavailable, context_tokens, ..Usage::default() });
            acp.start(&request(Mode::Ask, Some("s")));
            assert_eq!(acp.usage, Usage { measurement: Measurement::Unavailable, ..Usage::default() });
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
