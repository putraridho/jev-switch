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
fn consumption_reported_with_the_finished_prompt_is_used_and_half_reports_are_not() {
    let finish = |usage: Value| {
        let mut acp = Acp::for_agent(Agent::Antigravity);
        setup(&mut acp, Mode::Ask, None);
        acp.parse(&update(json!({"sessionUpdate":"usage_update","used":7_000})));
        acp.parse(&update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"done"}})));
        let events = acp.parse(&json!({"id":5,"result":{"stopReason":"end_turn","usage":usage}}));
        let Some(Event::Finished(finish)) = events.last() else { panic!("{events:?}") };
        finish.usage
    };
    assert_eq!(
        finish(json!({"inputTokens":1_000,"outputTokens":50,"cachedReadTokens":800})),
        Usage { measurement: Measurement::Reported, input_tokens: 1_000, cached_input_tokens: 800, output_tokens: 50, context_tokens: Some(7_000),
            cache_written_tokens: None,
            window_tokens: None }
    );
    assert_eq!(finish(json!({"inputTokens":10,"outputTokens":0,"cachedReadTokens":99})).cached_input_tokens, 10, "the cached share cannot exceed the input");
    for half in [json!({"inputTokens":1_000}), json!({"outputTokens":5}), json!({"inputTokens":"1","outputTokens":2}), json!(null), json!("x")] {
        let usage = finish(half);
        assert_eq!((usage.measurement, usage.input_tokens, usage.output_tokens), (Measurement::Unavailable, 0, 0));
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

#[test]
fn the_size_of_a_usage_update_is_the_agents_own_context_window() {
    let mut acp = Acp::for_agent(Agent::Antigravity);
    setup(&mut acp, Mode::Ask, None);
    acp.parse(&update(json!({"sessionUpdate":"usage_update","used":7_000,"size":1_048_576})));
    acp.parse(
        &update(
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"done"}})
        )
    );
    let events = acp.parse(
        &json!({"id":5,"result":{"stopReason":"end_turn","usage":{"inputTokens":10,"outputTokens":1}}})
    );
    let Some(Event::Finished(finish)) = events.last() else { panic!("{events:?}") };
    assert_eq!(
        (finish.usage.context_tokens, finish.usage.window_tokens),
        (Some(7_000), Some(1_048_576))
    );
}
