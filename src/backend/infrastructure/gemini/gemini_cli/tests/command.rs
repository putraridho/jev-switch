//! The `gemini` / `agy` command lines and the constructors.

use super::*;

    #[test]
    fn gemini_command_pipes_the_prompt_and_respects_mode() {
        let b = gemini();
        for (mode, approval) in [(Mode::Ask, "default"), (Mode::Plan, "default"), (Mode::Edit, "auto_edit"), (Mode::Auto, "auto_edit")] {
            let request = TurnRequest { mode, ..req(None) };
            assert_eq!(args(&b.command(&request)), ["-m", "m-1", "--output-format", "stream-json", "--approval-mode", approval]);
            let resumed = TurnRequest { mode, ..req(Some("sid")) };
            assert_eq!(args(&b.command(&resumed)), ["-m", "m-1", "--output-format", "stream-json", "--approval-mode", approval, "--resume", "sid"]);
        }
        assert_eq!(b.stdin_payload(&req(None)).as_deref(), Some("do the thing"));
    }

    #[test]
    fn agy_command_uses_acp_and_keeps_stdin_open_for_approvals() {
        let b = agy();
        assert_eq!(args(&b.command(&req(None))), ["acp"]);
        assert!(b.interactive());
        let init: Value = serde_json::from_str(&b.stdin_payload(&req(None)).unwrap()).unwrap();
        assert_eq!(init["method"], "initialize");
    }

    #[test]
    fn a_huge_prompt_never_becomes_an_oversized_argument() {
        let prompt = "q".repeat(500_000);
        let request = TurnRequest { prompt: prompt.clone(), ..req(Some("cid")) };
        let b = agy();
        assert_eq!(args(&b.command(&request)), ["acp"]);
        assert!(b.stdin_payload(&request).unwrap().len() < 500);
        let g = gemini();
        assert!(args(&g.command(&request)).iter().all(|a| a.len() < 100));
        assert_eq!(g.stdin_payload(&request).as_deref(), Some(prompt.as_str()));
    }

    #[test]
    fn no_approve_everything_flag_is_ever_passed() {
        let forbidden = ["--yolo", "-y", "yolo", "--dangerously-skip-permissions", "always-proceed"];
        for b in [gemini(), agy()] {
            for session in [None, Some("s")] {
                let all = args(&b.command(&req(session)));
                assert!(all.iter().all(|a| !forbidden.contains(&a.as_str())), "{:?}: {all:?}", b.flavor);
            }
        }
    }

    /// A gemini and an agy turn: fresh, then resumed.
    fn golden_cases(backend: &GeminiBackend) -> String {
        [("fresh", req(None)), ("resumed", req(Some("sess-1")))]
            .map(|(name, r)| format!("=== {name}\n{}\n", serde_json::to_string(&args(&backend.command(&r))).unwrap()))
            .concat()
    }

    #[test]
    fn golden_gemini_args() {
        const MASKED: &str = "# masked: nothing";
        const GOLDEN: &str = golden!("backend/gemini_args.txt");
        let actual = golden_cases(&gemini());
        let (head, body) = GOLDEN.split_once('\n').unwrap();
        assert_eq!(head, MASKED);
        assert_eq!(body, actual);
    }

    #[test]
    fn golden_agy_args() {
        const MASKED: &str = "# masked: nothing";
        const GOLDEN: &str = golden!("backend/agy_args.txt");
        let actual = golden_cases(&agy());
        let (head, body) = GOLDEN.split_once('\n').unwrap();
        assert_eq!(head, MASKED);
        assert_eq!(body, actual);
    }

    #[test]
    fn command_runs_in_the_request_dir_with_the_configured_binary() {
        let b = GeminiBackend::new(Flavor::GeminiCli, "/opt/gemini");
        let cmd = b.command(&req(None));
        assert_eq!(cmd.as_std().get_program(), "/opt/gemini");
        assert_eq!(cmd.as_std().get_current_dir(), Some(std::path::Path::new("/tmp")));
    }

    #[test]
    fn extra_env_reaches_the_child_so_a_key_kept_in_the_env_file_works() {
        let b = GeminiBackend::new(Flavor::GeminiCli, "gemini").with_env(vec![("GEMINI_API_KEY".into(), "k".into())]);
        let cmd = b.command(&req(None));
        assert!(cmd.as_std().get_envs().any(|(k, v)| k == "GEMINI_API_KEY" && v == Some(std::ffi::OsStr::new("k"))));
    }

    #[test]
    fn both_flavors_are_the_gemini_provider() {
        assert_eq!(gemini().provider(), ProviderId::Gemini);
        assert_eq!(agy().provider(), ProviderId::Gemini);
        assert_eq!(GeminiBackend::gemini_cli().flavor, Flavor::GeminiCli);
        assert_eq!(GeminiBackend::antigravity().flavor, Flavor::Antigravity);
    }

    #[test]
    fn binary_falls_back_to_the_default_name() {
        assert_eq!(bin_from_env("JEV_TEST_UNSET_BIN_VAR", "gemini"), "gemini");
    }
