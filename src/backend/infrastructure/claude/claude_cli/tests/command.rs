//! The command line: arguments, environment, trust and the session note.

use super::*;
use crate::backend::infrastructure::claude::claude_cli::command::{setting_sources, NO_BACKGROUND_ENV};

#[test]
fn plan_mode_is_read_only_even_when_nobody_can_be_asked() {
    let headless = backend();
    let mut r = req(None);
    r.mode = crate::approval::domain::mode::Mode::Plan;
    let args: Vec<String> = headless.command(&r).as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert!(args.windows(2).any(|w| w == ["--permission-mode", "plan"]), "{args:?}");
}

#[test]
fn partial_messages_are_requested_only_when_streaming() {
    let mut b = backend();
    let plain = args(&b.command(&req(None)));
    assert!(!plain.iter().any(|a| a == "--include-partial-messages"), "{plain:?}");
    b.set_streaming(true);
    let streamed = args(&b.command(&req(None)));
    assert!(streamed.iter().any(|a| a == "--include-partial-messages"), "{streamed:?}");
}

#[test]
fn command_matches_the_old_inline_invocation() {
    assert_eq!(
        args(&backend().command(&req(None))),
        ["-p", "--model", "sonnet-x", "--output-format", "stream-json", "--verbose", "--setting-sources", "user,local", "--permission-mode", "acceptEdits"]
    );
    let b = ClaudeBackend { allowed_tools: Some("Bash(cargo *)".into()), ..backend() };
    assert_eq!(
        args(&b.command(&req(Some("abc")))),
        ["-p", "--model", "sonnet-x", "--output-format", "stream-json", "--verbose", "--resume", "abc",
         "--setting-sources", "user,local", "--permission-mode", "acceptEdits", "--allowedTools", "Bash(cargo *)"]
    );
}

#[test]
fn every_run_follows_the_requested_mode() {
    use crate::approval::domain::mode::Mode;
    let session = backend().in_session();
    for (mode, flag) in [(Mode::Ask, "default"), (Mode::Edit, "acceptEdits"), (Mode::Plan, "plan"), (Mode::Auto, "acceptEdits")] {
        for b in [&session, &backend()] {
            let a = args(&b.command(&TurnRequest { mode, ..req(None) }));
            let at = a.iter().position(|x| x == "--permission-mode").unwrap();
            assert_eq!(a[at + 1], flag, "{mode}");
        }
    }
}

#[test]
fn setting_sources_respect_project_and_local_trust() {
    assert_eq!(setting_sources(false, false), "user");
    assert_eq!(setting_sources(true, false), "user,local");
    assert_eq!(setting_sources(false, true), "user,project");
    assert_eq!(setting_sources(true, true), "user,project,local");
}

#[test]
fn the_callers_trust_decision_decides_whether_project_settings_are_read() {
    let b = backend();
    let sources = |trusted: bool| {
        let a = args(&b.command(&TurnRequest { trust_project_settings: trusted, ..req(None) }));
        a[a.iter().position(|x| x == "--setting-sources").unwrap() + 1].clone()
    };
    assert!(!sources(false).contains("project"));
    assert!(sources(true).contains("project"));
}

#[test]
fn attached_mcp_config_is_passed_and_its_tools_allowed() {
    let mut b = backend();
    assert!(b.supports_mcp());
    b.attach_mcp(&attachment());
    let a = args(&b.command(&req(None)));
    let i = a.iter().position(|x| x == "--mcp-config").expect("flag");
    assert_eq!(a[i + 1], "/state/mcp.json");
    let j = a.iter().position(|x| x == "--allowedTools").expect("server allowed");
    assert_eq!(a[j + 1], "mcp__jev");
    let mut b = ClaudeBackend { allowed_tools: Some("Bash(cargo *)".into()), ..backend() };
    b.attach_mcp(&attachment());
    let a = args(&b.command(&req(None)));
    assert_eq!(a[a.iter().position(|x| x == "--allowedTools").unwrap() + 1], "Bash(cargo *) mcp__jev");
    for mode in [crate::approval::domain::mode::Mode::Ask, crate::approval::domain::mode::Mode::Plan] {
        let a = args(&b.command(&TurnRequest { mode, ..req(None) }));
        assert_eq!(a[a.iter().position(|x| x == "--allowedTools").unwrap() + 1], "Bash(cargo *)");
        assert_eq!(a.iter().any(|x| x == "--mcp-config"), mode == crate::approval::domain::mode::Mode::Ask);
    }
    let a = args(&b.command(&TurnRequest { mode: crate::approval::domain::mode::Mode::Auto, ..req(None) }));
    assert_eq!(a[a.iter().position(|x| x == "--allowedTools").unwrap() + 1], "Bash(cargo *) mcp__jev");
    // Without an attachment the command is the one it always was.
    assert!(!args(&backend().command(&req(None))).iter().any(|x| x.contains("mcp")));
}

#[test]
fn every_run_turns_off_background_tasks_that_would_outlive_the_turn() {
    for b in [backend(), session()] {
        let cmd = b.command(&req(None));
        let set: Vec<_> = cmd.as_std().get_envs().filter(|(k, _)| *k == NO_BACKGROUND_ENV).collect();
        assert_eq!(set.len(), 1);
        assert_eq!(set[0].1.and_then(|v| v.to_str()), Some("1"));
    }
}

#[test]
fn effort_sets_the_thinking_budget_in_the_environment_once() {
    let mut b = backend();
    b.set_effort(crate::backend::Effort::Low);
    b.set_effort(crate::backend::Effort::High);
    let cmd = b.command(&req(None));
    let budgets: Vec<_> = cmd.as_std().get_envs().filter(|(k, _)| *k == "MAX_THINKING_TOKENS").collect();
    assert_eq!(budgets.len(), 1);
    assert_eq!(budgets[0].1.and_then(|v| v.to_str()), Some("31999"));
    assert!(backend().command(&req(None)).as_std().get_envs().all(|(k, _)| k != "MAX_THINKING_TOKENS"));
}

#[test]
fn profile_env_reaches_the_command() {
    let b = ClaudeBackend { env: vec![("ANTHROPIC_BASE_URL".into(), "https://x".into())], provider: ProviderId::DeepSeek, ..backend() };
    let cmd = b.command(&req(None));
    let envs: Vec<_> = cmd.as_std().get_envs().collect();
    assert!(envs.iter().any(|(k, v)| *k == "ANTHROPIC_BASE_URL" && v.is_some_and(|v| v == "https://x")));
    assert_eq!(b.provider(), ProviderId::DeepSeek);
}

#[test]
fn golden_claude_args() {
    const MASKED: &str = "# masked: the value after --setting-sources (<SOURCES>, from the developer's trust settings)";
    const GOLDEN: &str = golden!("backend/claude_args.txt");
    let with_tools = ClaudeBackend { allowed_tools: Some("Bash(cargo *)".into()), ..backend() };
    let mut mcp = session();
    mcp.attach_mcp(&attachment());
    let actual = [
        golden_case("fresh", &backend().command(&req(None))),
        golden_case("resumed", &with_tools.command(&req(Some("sess-1")))),
        golden_case("session_with_mcp", &mcp.command(&req(None))),
    ]
    .concat();
    let (head, body) = GOLDEN.split_once('\n').unwrap();
    assert_eq!(head, MASKED);
    assert_eq!(body, actual);
}

#[test]
fn golden_deepseek_args() {
    const MASKED: &str = "# masked: the value after --setting-sources (<SOURCES>, from the developer's trust settings)";
    const GOLDEN: &str = golden!("backend/deepseek_args.txt");
    let deepseek = ClaudeBackend {
        provider: ProviderId::DeepSeek,
        env: vec![("ANTHROPIC_BASE_URL".into(), "https://deepseek.test/anthropic".into())],
        ..backend()
    };
    let actual = [
        golden_case("fresh", &deepseek.command(&req(None))),
        golden_case("resumed_in_session", &ClaudeBackend { interactive: true, ..deepseek }.command(&req(Some("sess-1")))),
    ]
    .concat();
    let (head, body) = GOLDEN.split_once('\n').unwrap();
    assert_eq!(head, MASKED);
    assert_eq!(body, actual);
}

#[test]
fn a_session_runs_with_stream_json_input_a_host_prompt_tool_and_the_note() {
    let a = args(&session().command(&req(None)));
    assert_eq!(
        a,
        ["-p", "--model", "sonnet-x", "--output-format", "stream-json", "--verbose", "--setting-sources", "user,local", "--permission-mode", "acceptEdits",
         "--input-format", "stream-json", "--replay-user-messages", "--permission-prompt-tool", "stdio", "--disallowedTools", "AskUserQuestion",
         "--append-system-prompt", SESSION_NOTE]
    );
    assert!(session().interactive());
    // Headless use is what it always was.
    assert!(!backend().interactive());
    assert!(!args(&backend().command(&req(None))).iter().any(|x| x.contains("input-format") || x.contains("system-prompt")));
}

#[test]
fn the_note_is_short_stable_and_steers_away_from_claude_code_ui() {
    assert!(SESSION_NOTE.len() < 900, "{}", SESSION_NOTE.len());
    assert!(SESSION_NOTE.contains("jev-code") && SESSION_NOTE.contains("/permissions") && SESSION_NOTE.contains("Shift+Tab"));
    assert!(!SESSION_NOTE.contains('\n'), "no stray line breaks from the source layout");
    let a = args(&session().command(&req(Some("abc"))));
    let b = args(&session().command(&req(None)));
    let note = |v: &[String]| v[v.iter().position(|x| x == "--append-system-prompt").unwrap() + 1].clone();
    assert_eq!(note(&a), note(&b), "byte-identical every turn");
}
