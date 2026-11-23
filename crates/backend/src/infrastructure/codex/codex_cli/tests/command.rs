//! The `codex` command lines, sandbox, MCP overrides and images.

use super::*;

    #[test]
    fn image_arguments_for_fresh_and_resume() {
        let args = |request: &TurnRequest| -> Vec<String> { backend().command(request).as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect() };
        let mut fresh = req(None);
        fresh.images = vec!["/tmp/picture.png".into(), "/tmp/shot 10,30.png".into()];
        assert!(args(&fresh).contains(&"--image=/tmp/picture.png".to_string()));
        assert!(!args(&fresh).iter().any(|a| a.contains("10,30")), "a path with a comma is not a flag value");
        let payload = backend().stdin_payload(&fresh).unwrap_or_default();
        assert!(payload.contains("/tmp/shot 10,30.png") && !payload.contains("/tmp/picture.png"), "{payload}");

        let mut resumed = req(Some("thread"));
        resumed.images = vec!["/tmp/picture.png".into()];
        let a = args(&resumed);
        assert!(!a.iter().any(|x| x.starts_with("--image")), "{a:?}");
        assert_eq!(&a[a.len() - 2..], ["thread", "-"], "the thread id stays the thread id");
        assert!(backend().stdin_payload(&resumed).unwrap_or_default().contains("/tmp/picture.png"));
    }

    #[test]
    fn plan_mode_runs_codex_read_only() {
        let args = |request: &TurnRequest| -> Vec<String> { backend().command(request).as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect() };
        for session in [None, Some("thread")] {
            let mut r = req(session);
            r.mode = jev_approval::domain::mode::Mode::Plan;
            let a = args(&r);
            assert!(a.windows(2).any(|w| w == ["-s", "read-only"]) || a.contains(&"sandbox_mode=\"read-only\"".to_string()), "{a:?}");
        }
    }

    #[test]
    fn effort_is_passed_as_a_config_override_on_new_and_resumed_turns_and_only_when_set() {
        let args = |b: &CodexBackend, request: &TurnRequest| -> Vec<String> { b.command(request).as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect() };
        let mut set = backend();
        set.set_effort(crate::Effort::High);
        for session in [None, Some("thread")] {
            assert!(args(&set, &req(session)).windows(2).any(|w| w == ["-c", "model_reasoning_effort=\"high\""]));
            assert!(!args(&backend(), &req(session)).iter().any(|a| a.contains("model_reasoning_effort")));
        }
    }

    #[test]
    fn new_turn_command_sets_dir_sandbox_and_model() {
        let cmd = backend().command(&req(None));
        assert_eq!(args(&cmd), ["exec", "--json", "-C", "/tmp/proj", "-s", "workspace-write", "-m", "gpt-x"]);
        assert_eq!(cmd.as_std().get_current_dir(), Some(std::path::Path::new("/tmp/proj")));
        assert_eq!(cmd.as_std().get_program(), "codex");
    }

    #[test]
    fn resume_command_reads_the_prompt_from_stdin_and_keeps_the_sandbox() {
        let cmd = backend().command(&req(Some("thread-1")));
        assert_eq!(
            args(&cmd),
            ["exec", "resume", "--json", "-m", "gpt-x", "-c", "sandbox_mode=\"workspace-write\"", "thread-1", "-"]
        );
        assert_eq!(cmd.as_std().get_current_dir(), Some(std::path::Path::new("/tmp/proj")));
    }

    #[test]
    fn prompt_travels_on_stdin() {
        assert_eq!(backend().stdin_payload(&req(None)).as_deref(), Some("hi"));
    }

    #[test]
    fn attached_mcp_server_goes_in_as_config_overrides_before_the_thread_id() {
        let mut b = CodexBackend::new("codex", "workspace-write");
        assert!(b.supports_mcp());
        assert!(!args(&b.command(&req(None))).iter().any(|a| a.contains("mcp_servers")));
        b.attach_mcp(&McpAttachment {
            name: "jev".into(),
            command: "/bin/delegate_mcp".into(),
            args: vec!["--dir".into(), "/my repo".into()],
            config_path: "/x.json".into(),
        });
        let fresh = args(&b.command(&req(None)));
        assert!(fresh.contains(&"mcp_servers.jev.command=\"/bin/delegate_mcp\"".to_string()), "{fresh:?}");
        assert!(fresh.contains(&"mcp_servers.jev.args=[\"--dir\",\"/my repo\"]".to_string()), "{fresh:?}");
        assert!(
            fresh.contains(&"mcp_servers.jev.tool_timeout_sec=3600".to_string()),
            "a delegate call outlives codex's 60 s default: {fresh:?}"
        );
        assert!(
            fresh.contains(&"mcp_servers.jev.default_tools_approval_mode=\"approve\"".to_string()),
            "headless codex refuses MCP tools unless this server's tools are pre-approved: {fresh:?}"
        );
        let resumed = args(&b.command(&req(Some("t1"))));
        assert_eq!(&resumed[resumed.len() - 2..], ["t1", "-"], "overrides stay before the positionals");
        assert!(resumed.iter().any(|a| a.starts_with("mcp_servers.jev.command")));
        for mode in [jev_approval::domain::mode::Mode::Ask, jev_approval::domain::mode::Mode::Edit, jev_approval::domain::mode::Mode::Plan, jev_approval::domain::mode::Mode::Auto] {
            for session in [None, Some("t1")] {
                let a = args(&b.command(&TurnRequest { mode, ..req(session) }));
                assert_eq!(a.iter().any(|arg| arg.contains("default_tools_approval_mode")), matches!(mode, jev_approval::domain::mode::Mode::Edit | jev_approval::domain::mode::Mode::Auto), "{mode}: {a:?}");
            }
        }
    }

    #[test]
    fn golden_codex_args() {
        const MASKED: &str = "# masked: nothing";
        const GOLDEN: &str = golden!("backend/codex_args.txt");
        let case = |name: &str, b: &CodexBackend, r: &TurnRequest| format!("=== {name}\n{}\n", serde_json::to_string(&args(&b.command(r))).unwrap());
        let mut with_mcp = backend();
        with_mcp.attach_mcp(&McpAttachment {
            name: "jev".into(),
            command: "/bin/delegate_mcp".into(),
            args: vec!["--dir".into(), "/repo".into()],
            config_path: "/state/mcp.json".into(),
        });
        let mut images = req(None);
        images.images = vec!["/tmp/proj/shot.png".into()];
        let actual = [
            case("fresh", &backend(), &req(None)),
            case("fresh_with_image", &backend(), &images),
            case("resumed", &backend(), &req(Some("thread-1"))),
            case("resumed_with_mcp", &with_mcp, &req(Some("thread-1"))),
        ]
        .concat();
        let (head, body) = GOLDEN.split_once('\n').unwrap();
        assert_eq!(head, MASKED);
        assert_eq!(body, actual);
    }

    #[test]
    fn command_never_asks_for_less_sandbox_whatever_the_env_says() {
        for junk in ["danger-full-access", "", "READ-ONLY", "workspace-write --dangerously-bypass-approvals-and-sandbox", "bogus"] {
            let b = CodexBackend::new("codex", junk);
            assert_eq!(b.sandbox_arg(), "workspace-write", "{junk:?}");
            for session in [None, Some("t")] {
                let all = args(&b.command(&req(session))).join(" ");
                assert!(!all.contains("danger"), "{all}");
                assert!(!all.contains("bypass"), "{all}");
            }
        }
        assert_eq!(CodexBackend::new("codex", "read-only").sandbox_arg(), "read-only");
    }

    #[test]
    fn provider_is_codex() {
        assert_eq!(backend().provider(), ProviderId::Codex);
    }

    #[test]
    fn network_is_opened_only_when_asked_and_only_in_workspace_write() {
        let args = |b: &CodexBackend, request: &TurnRequest| -> Vec<String> { b.command(request).as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect() };
        let has = |a: &[String]| a.windows(2).any(|w| w == ["-c", "sandbox_workspace_write.network_access=true"]);
        let on = backend().with_network(true);
        for session in [None, Some("thread")] {
            assert!(!has(&args(&backend(), &req(session))), "off by default");
            assert!(has(&args(&on, &req(session))), "new and resumed turns");
            let mut plan = req(session);
            plan.mode = jev_approval::domain::mode::Mode::Plan;
            assert!(!has(&args(&on, &plan)), "plan mode is read-only and stays closed");
        }
        assert!(!has(&args(&CodexBackend::new("codex", "read-only").with_network(true), &req(None))), "a read-only sandbox stays closed");
    }
