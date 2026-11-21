use super::*;

#[test]
fn a_blocked_path_in_the_prompt_refuses_those_providers_only() {
    let p = policy("block secrets/** from deepseek,grok\nblock *.pem from all");
    let v = check(&p, "look at @secrets/db.yml and fix it");
    assert!(!v.permits(ProviderId::DeepSeek) && !v.permits(ProviderId::Grok));
    assert!(v.permits(ProviderId::Claude) && v.permits(ProviderId::Codex));
    assert_eq!(v.denial(ProviderId::Grok).as_deref(), Some("secrets/db.yml is covered by `block secrets/** from deepseek,grok`"));
    let v = check(&p, "rotate \"deploy/a.pem\".");
    assert!(jev_shared::ALL_PROVIDERS.iter().all(|p| !v.permits(*p)));
    assert!(v.refusal_line().starts_with("privacy policy: deploy/a.pem"));
    // Absolute paths, ./ and :line suffixes still hit.
    assert!(!check(&p, "see /work/app/secrets/x.rs:12").permits(ProviderId::Grok));
    assert!(!check(&p, "see ./secrets/x.rs").permits(ProviderId::Grok));
    // A prompt that names nothing sensitive is open.
    assert_eq!(check(&p, "explain the parser"), Verdict::open());
}

#[test]
fn only_rules_allow_a_list() {
    let p = policy("only claude,codex for ops/**");
    let v = check(&p, "edit ops/deploy.sh");
    assert!(v.permits(ProviderId::Claude) && v.permits(ProviderId::Codex));
    assert!(!v.permits(ProviderId::Gemini) && !v.permits(ProviderId::DeepSeek));
}

#[test]
fn attached_files_count() {
    let p = policy("block *.png from all");
    let v = p.check(Path::new(ROOT), "what is this", &[PathBuf::from("/work/app/shots/a.png")], None);
    assert!(!v.permits(ProviderId::Claude));
}

#[test]
fn local_only_refuses_every_cloud_provider() {
    let p = policy("local-only on");
    let v = check(&p, "hello");
    assert!(jev_shared::ALL_PROVIDERS.iter().all(|p| !v.permits(*p)));
    assert!(v.refusal_line().contains("local-only"));
    assert!(Policy::default().with_local_only(true).local_only());
}
