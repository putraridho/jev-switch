use super::*;

#[test]
fn model_ids_are_unique_and_chains_resolve() {
    let mut ids: Vec<_> = models().iter().map(|m| m.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), models().len());
    for t in TaskType::ALL {
        for vh in [false, true] {
            assert!(!default_chain(t, vh).is_empty(), "{t} very_hard={vh}");
        }
    }
}

#[test]
fn task_type_round_trips() {
    for t in TaskType::ALL {
        assert_eq!(t.as_str().parse::<TaskType>().unwrap(), t);
    }
    assert!("nope".parse::<TaskType>().is_err());
}

#[test]
fn very_hard_only_models_need_very_hard() {
    for t in TaskType::ALL {
        assert!(default_chain(t, false).iter().all(|s| !s.very_hard_only));
    }
    let hard = default_chain(TaskType::HardDebug, true);
    assert_eq!(hard[0].id, "gpt-6-astra");
    assert!(default_chain(TaskType::HardDebug, false).iter().all(|s| s.id != "gpt-6-astra"));
}

#[test]
fn allowed_for_blocks_security_on_deepseek_and_grok_for_every_task() {
    for spec in models() {
        for t in TaskType::ALL {
            let blocked = t == TaskType::Security && matches!(spec.provider, DeepSeek | Grok);
            assert_eq!(allowed_for(t, spec), !blocked, "{t} {}", spec.id);
        }
    }
    for t in TaskType::ALL {
        for vh in [false, true] {
            for s in default_chain(t, vh) {
                assert!(allowed_for(t, s));
            }
        }
    }
}
