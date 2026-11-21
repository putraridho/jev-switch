use super::*;
use jev_shared::domain::secrets::SecretKind;
use crate::domain::privacy::outgoing::prepare;

#[test]
fn prepare_blanks_secrets_unless_redaction_is_off() {
    let text = "use DB_PASSWORD=hunter2hunter2 here";
    let on = prepare(&Policy::default(), text);
    assert_eq!(on.prompt, "use DB_PASSWORD=[REDACTED:named-secret] here");
    assert_eq!(on.redacted, vec![SecretKind::NamedSecret]);
    let off = prepare(&policy("redact off"), text);
    assert_eq!(off.prompt, text);
    assert!(off.redacted.is_empty());
}
