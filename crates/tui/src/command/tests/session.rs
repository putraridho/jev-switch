//! Session, info and permission commands.

use super::*;

#[test]
fn session_and_info_commands() {
    assert_eq!(parse("/help"), Command::Help);
    assert_eq!(parse("/status"), Command::Status);
    assert_eq!(parse("/doctor"), Command::Doctor);
    assert_eq!(parse("/update"), Command::Update);
    assert!(matches!(parse("/update now"), Command::Invalid(_)));
    assert_eq!(parse("/undo"), Command::Undo);
    assert_eq!(parse("/rewind"), Command::Rewind(None));
    assert_eq!(parse("/rewind 2"), Command::Rewind(Some(2)));
    assert!(matches!(parse("/rewind 0"), Command::Invalid(_)));
    assert!(matches!(parse("/rewind x"), Command::Invalid(_)));
    assert!(matches!(parse("/rewind 1 2"), Command::Invalid(_)));
    assert_eq!(parse("/resume"), Command::Resume(None));
    assert_eq!(parse("/rename parser fix"), Command::Rename(Some("parser fix".into())));
    assert_eq!(parse("/rename"), Command::Rename(None));
    assert_eq!(parse("/workflows"), Command::Workflows);
    assert_eq!(parse("/workers"), Command::Workflows);
    assert_eq!(parse("/resume fix parser"), Command::Resume(Some("fix parser".into())));
    assert_eq!(parse("/HELP"), Command::Help, "commands are not case sensitive");
    assert_eq!(parse("/help me"), Command::Learn(crate::views::learn::Learn::Search("me".into())));
    assert_eq!(parse("/undo now"), Command::Invalid("usage: /undo".into()));
}

#[test]
fn permissions_takes_the_rest_of_the_line_as_the_rule() {
    assert_eq!(parse("/permissions"), Command::Permissions(PermissionsCmd::List));
    assert_eq!(parse("/Permissions allow Bash(cargo test:*)"), Command::Permissions(PermissionsCmd::Allow("Bash(cargo test:*)".into())));
    assert_eq!(parse("/permissions remove Bash(ls:*)"), Command::Permissions(PermissionsCmd::Remove("Bash(ls:*)".into())));
    assert_eq!(parse("/permissions dir ~/data"), Command::Permissions(PermissionsCmd::Dir("~/data".into())));
    assert!(matches!(parse("/permissions allow"), Command::Invalid(m) if m.contains("usage: /permissions")));
    assert!(matches!(parse("/permissions allow nonsense("), Command::Invalid(m) if m.contains("not a permission rule")));
}

#[test]
fn new_session_and_context_commands() {
    assert_eq!(parse("/clear"), Command::Clear);
    assert_eq!(parse("/compact"), Command::Compact(None));
    assert_eq!(parse("/compact keep the API notes"), Command::Compact(Some("keep the API notes".into())));
    assert_eq!(parse("/context"), Command::Context);
    assert_eq!(parse("/cost"), Command::Cost);
    assert_eq!(parse("/usage"), Command::Cost);
    assert_eq!(parse("/usage now"), Command::Invalid("usage: /cost".into()));
    assert_eq!(parse("/why"), Command::Why);
    assert_eq!(parse("/cache"), Command::Cache);
    assert_eq!(parse("/cache now"), Command::Invalid("usage: /cache".into()));
    assert_eq!(parse("/effort"), Command::Effort(None));
    assert_eq!(parse("/effort High"), Command::Effort(Some(jev_turn::domain::effort::EffortSetting::Fixed(jev_backend::Effort::High))));
    assert!(matches!(parse("/effort max"), Command::Invalid(m) if m.contains("usage: /effort")));
    assert!(matches!(parse("/effort low now"), Command::Invalid(_)));
    assert_eq!(parse("/budget usd 5"), Command::Guard(GuardCmd::BudgetSet(jev_turn::domain::budget::Cap::Usd(5.0))));
    assert_eq!(parse("/privacy audit"), Command::Guard(GuardCmd::Audit));
    assert!(matches!(parse("/privacy nope"), Command::Invalid(_)));
    assert_eq!(parse("/export"), Command::Export(None));
    assert_eq!(parse("/export notes/session.md"), Command::Export(Some("notes/session.md".into())));
    assert_eq!(parse("/copy"), Command::Copy);
    assert_eq!(parse("/edit"), Command::Edit);
    assert_eq!(parse("/init"), Command::Init);
    assert_eq!(parse("/memory"), Command::Memory);
    assert_eq!(parse("/review"), Command::Review(None));
    assert_eq!(parse("/review only the parser"), Command::Review(Some("only the parser".into())));
    assert_eq!(parse("/config"), Command::Config);
    assert_eq!(parse("/agents"), Command::Agents);
    assert_eq!(parse("/orchestration"), Command::Orchestration);
    assert!(matches!(parse("/agents now"), Command::Invalid(_)));
    assert_eq!(parse("/terminal-setup"), Command::TerminalSetup { install: false });
    assert_eq!(parse("/terminal-setup install"), Command::TerminalSetup { install: true });
    assert!(matches!(parse("/terminal-setup now"), Command::Invalid(_)));
    assert_eq!(parse("/clear all"), Command::Invalid("usage: /clear".into()));
}

#[test]
fn diff_takes_the_rest_of_the_line_as_one_path() {
    assert_eq!(parse("/diff"), Command::Diff(None));
    assert_eq!(parse("/diff src/main.rs"), Command::Diff(Some("src/main.rs".into())));
    assert_eq!(parse("/diff  my notes/a b.md "), Command::Diff(Some("my notes/a b.md".into())));
}
