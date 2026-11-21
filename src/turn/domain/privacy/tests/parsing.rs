use super::*;

#[test]
fn bad_lines_are_reported_and_the_rest_applies() {
    let (p, errors) = Policy::parse("redact maybe\nblock x from nobody\nfoo\nlocal-only on # now\n\n# c", Source::User);
    assert_eq!(errors.iter().map(|e| e.line).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert!(p.local_only());
    assert_eq!(p.rule_count(), 0);
}

#[test]
fn unknown_providers_are_reported_without_dropping_known_ones() {
    let (p, errors) = Policy::parse("block secret from grok,unknown\nonly claude,unknown for ops/**\nblock other from unknown\nonly unknown for private/**", Source::User);
    assert_eq!(errors.iter().map(|error| error.line).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
    assert!(errors.iter().all(|error| error.message.contains("unknown provider")));
    assert_eq!(p.rule_count(), 2);
    assert!(!check(&p, "secret").permits(ProviderId::Grok));
    assert!(check(&p, "secret").permits(ProviderId::Claude));
    assert!(check(&p, "ops/file").permits(ProviderId::Claude));
    assert!(!check(&p, "ops/file").permits(ProviderId::Grok));
}

#[test]
fn a_project_file_cannot_loosen_the_policy() {
    let (p, errors) = Policy::parse("redact off", Source::Project);
    assert_eq!(errors.len(), 1);
    assert!(p.redacts());
    let (user, _) = Policy::parse("redact off", Source::User);
    assert!(!user.redacts());
    // Merging: the user's `off` is not undone by a file that says nothing.
    assert!(!user.clone().merged(Policy::default()).redacts());
    assert!(!Policy::default().merged(user).redacts());
}
