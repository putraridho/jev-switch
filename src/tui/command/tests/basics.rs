//! Plain text, shell lines, unknown words and the command table.

use super::*;

#[test]
fn plain_text_is_a_prompt_and_blank_is_empty() {
    assert_eq!(parse("fix the bug"), Command::Prompt("fix the bug".into()));
    assert_eq!(parse("   "), Command::Empty);
    assert_eq!(parse("/etc/hosts looks odd"), Command::Prompt("/etc/hosts looks odd".into()));
}

#[test]
fn simple_commands() {
    assert_eq!(parse("/quit"), Command::Quit);
    assert_eq!(parse("/exit"), Command::Quit);
    assert_eq!(parse("/new"), Command::New);
    assert_eq!(parse("/providers"), Command::Providers { refresh: false });
    assert_eq!(parse("/providers refresh"), Command::Providers { refresh: true });
    assert!(matches!(parse("/providers nope"), Command::Invalid(_)));
    assert_eq!(parse("/mcp"), Command::Mcp(McpCmd::Browse));
    assert_eq!(parse("/mcp list"), Command::Mcp(McpCmd::List));
    assert_eq!(parse("/mcp login my-server"), Command::Mcp(McpCmd::Login("my-server".into())));
    assert_eq!(parse("/mcp auth my-server"), Command::Mcp(McpCmd::Login("my-server".into())));
    assert_eq!(parse("/mcp login"), Command::Mcp(McpCmd::Browse));
    assert!(matches!(parse("/mcp login a b"), Command::Invalid(_)));
    assert!(matches!(parse("/mcp nope"), Command::Invalid(_)));
}

#[test]
fn unknown_slash_command_is_reported_not_sent() {
    assert_eq!(parse("/provders"), Command::Unknown { typed: "/provders".into(), args: String::new(), suggestion: Some("/providers") });
    assert_eq!(parse("/xyzzy"), Command::Unknown { typed: "/xyzzy".into(), args: String::new(), suggestion: None });
}

#[test]
fn a_bang_runs_a_shell_command() {
    assert_eq!(parse("# remember x"), Command::Note("remember x".into()));
    assert_eq!(parse("## Heading"), Command::Prompt("## Heading".into()));
    assert_eq!(parse("!git status"), Command::Shell("git status".into()));
    assert_eq!(parse("  ! ls -la "), Command::Shell("ls -la".into()));
    assert!(matches!(parse("!"), Command::Invalid(m) if m.contains("!git status")));
}

#[test]
fn every_command_in_the_table_is_understood() {
    // The table in uifmt drives help and completion; parsing must know each of its names.
    for name in uifmt::command_names() {
        assert!(!matches!(parse(name), Command::Unknown { .. } | Command::Prompt(_)), "{name}");
    }
}
