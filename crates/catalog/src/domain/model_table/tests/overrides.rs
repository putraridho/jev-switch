use super::*;

fn chains(entries: &[(&str, &[&str])]) -> Overrides {
    Overrides::from_raw(entries.iter().map(|(task, refs)| (task.to_string(), RawChain::of(refs))).collect())
}

#[test]
fn overrides_cannot_route_security_to_deepseek() {
    let o = chains(&[("security", &["deepseek:deepseek-v4-pro", "grok:grok-4.7", "claude:claude-opus-5-5"])]);
    let chain = chain_with(TaskType::Security, true, &o);
    assert_eq!(chain.iter().map(|s| s.id).collect::<Vec<_>>(), ["claude-opus-5-5"]);
}

#[test]
fn overrides_replace_defaults_and_report_unknowns() {
    let raw: Vec<(String, RawChain)> = vec![
        ("feature".into(), RawChain::of(&["claude:claude-haiku-5-5", "claude:nope", "bogus:x", "grok-4.7"])),
        ("weird".into(), RawChain::of(&["claude:claude-opus-5-5"])),
        ("docs".into(), RawChain::NotAList),
    ];
    let o = Overrides::from_raw(raw);
    assert_eq!(o.notes.len(), 5, "{:?}", o.notes);
    let chain = chain_with(TaskType::Feature, false, &o);
    assert_eq!(chain.iter().map(|s| s.id).collect::<Vec<_>>(), ["claude-haiku-5-5"]);
    // Unlisted task types keep their defaults.
    assert_eq!(chain_with(TaskType::Review, false, &o), default_chain(TaskType::Review, false));
}
