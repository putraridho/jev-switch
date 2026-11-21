use super::*;

    #[test]
    fn tool_rows_are_dim_with_the_name_plain_and_paths_relative() {
        let row = tool_row("Read /p/src/a.rs", Path::new("/p"), OFF, 80);
        assert_eq!(row, "  ⏺ Read src/a.rs");
        let styled = tool_row("Read /p/src/a.rs", Path::new("/p"), ON, 80);
        assert!(styled.contains("Read") && styled.contains("\x1b[2msrc/a.rs"));
        assert!(crate::tui::render::display_width(&tool_row(&"x".repeat(300), Path::new("/p"), OFF, 40)) <= 40);
    }


    #[test]
    fn delegation_has_its_own_mark_and_a_worker_result_says_who_did_what() {
        let row = tool_row("mcp__jev__delegate write the parser tests", Path::new("/p"), OFF, 80);
        assert_eq!(row, "  ⇉ Delegate write the parser tests");
        assert_eq!(tool_row("mcp__github__create_issue x", Path::new("/p"), OFF, 80), "  ⏺ github·create_issue x");
        let done = "worker w1 (gemini gemini-4-argon): done\nfiles changed (2):\n  a.rs +3 −1\nworker report:\nok";
        assert_eq!(tool_result_row("mcp__jev__delegate", true, done, OFF, 80).as_deref(), Some("    ⎿ ✓ worker w1 · gemini gemini-4-argon · done · 2 files"));
        let failed = "worker w2 (codex gpt-6): FAILED\nerror: limit";
        let row = tool_result_row("mcp__jev__delegate", true, failed, ON, 80).unwrap_or_default();
        assert!(row.contains("\x1b[31mFAILED"), "{row:?}");
        assert_eq!(display_tool("mcp__jev__show_diff"), "Show diff");
    }
