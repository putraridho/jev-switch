use super::*;

#[test]
fn overrides_cannot_route_security_to_deepseek() {
    let o = load_overrides(r#"{"security":["deepseek:deepseek-v4-pro","grok:grok-4.7","claude:claude-opus-5-5"]}"#)
        .unwrap();
    let chain = chain_with(TaskType::Security, true, &o);
    assert_eq!(chain.iter().map(|s| s.id).collect::<Vec<_>>(), ["claude-opus-5-5"]);
}

#[test]
fn overrides_replace_defaults_and_report_unknowns() {
    let o = load_overrides(
        r#"{"feature":["claude:claude-haiku-5-5","claude:nope","bogus:x","grok-4.7"],"weird":["claude:claude-opus-5-5"],"docs":"x"}"#,
    )
    .unwrap();
    assert_eq!(o.notes.len(), 5, "{:?}", o.notes);
    let chain = chain_with(TaskType::Feature, false, &o);
    assert_eq!(chain.iter().map(|s| s.id).collect::<Vec<_>>(), ["claude-haiku-5-5"]);
    // Unlisted task types keep their defaults.
    assert_eq!(chain_with(TaskType::Review, false, &o), default_chain(TaskType::Review, false));
    assert!(load_overrides("[1]").is_err());
    assert!(load_overrides("not json").is_err());
}
