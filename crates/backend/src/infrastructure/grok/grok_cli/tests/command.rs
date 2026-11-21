use super::*;
use crate::PromptFiles;

#[test]
fn golden_grok_args() {
    const MASKED: &str = "# masked: nothing";
    const GOLDEN: &str = golden!("backend/grok_args.txt");
    let case = |name: &str, r: TurnRequest| format!("=== {name}\n{}\n", serde_json::to_string(&args(&backend().command(&r))).unwrap());
    let actual = [
        case("fresh_edit", req(None)),
        case("resumed_edit", req(Some("sess-1"))),
        case("fresh_ask", TurnRequest { mode: Mode::Ask, ..req(None) }),
    ]
    .concat();
    let (head, body) = GOLDEN.split_once('\n').unwrap();
    assert_eq!(head, MASKED);
    assert_eq!(body, actual);
}

#[test]
fn command_is_headless_with_scoped_permissions() {
    for mode in Mode::ALL {
        let mut expected = vec!["--single=hi", "--model", "grok-4.6", "--output-format", "streaming-messages-json", "--no-auto-update", "--no-alt-screen", "--permission-mode", "default"];
        if matches!(mode, Mode::Edit | Mode::Auto) {
            expected.extend(["--allow", "Edit", "--allow", "Write"]);
        }
        assert_eq!(args(&backend().command(&TurnRequest { mode, ..req(None) })), expected);
        expected.extend(["--resume", "abc"]);
        assert_eq!(args(&backend().command(&TurnRequest { mode, ..req(Some("abc")) })), expected);
    }
}


#[test]
fn command_never_bypasses_permissions() {
    let all = args(&backend().command(&req(Some("abc")))).join(" ").to_lowercase();
    for banned in ["always-approve", "yolo", "bypasspermissions"] {
        assert!(!all.contains(banned), "{banned} in {all}");
    }
}

#[test]
fn prompt_travels_in_args_not_stdin() {
    let r = TurnRequest { prompt: "--looks-like-a-flag".into(), ..req(None) };
    assert_eq!(backend().stdin_payload(&r), None);
    assert_eq!(args(&backend().command(&r))[0], "--single=--looks-like-a-flag");
}

#[test]
fn a_huge_prompt_goes_through_prompt_file_and_no_argument_is_over_the_limit() {
    let dir = std::env::temp_dir().join(format!("jev-grok-prompt-{}", std::process::id()));
    let mut b = backend();
    b.prompts = PromptFiles::in_dir(dir.clone());
    let prompt = "q".repeat(500_000);
    let all = args(&b.command(&TurnRequest { prompt: prompt.clone(), ..req(None) }));
    assert!(all.iter().all(|a| a.len() <= crate::MAX_PROMPT_ARG_BYTES), "an argument is too long");
    assert_eq!(all[0], "--prompt-file");
    let path = std::path::PathBuf::from(&all[1]);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), prompt);
    assert!(!all.iter().any(|a| a.starts_with("--single")));
    b.cleanup();
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
