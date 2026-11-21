use super::*;

    #[test]
    fn a_run_of_calls_reads_as_one_sentence() {
        let mut g = ToolGroup::default();
        assert!(g.is_empty() && tool_group_row(&g, OFF, 80).is_empty());
        for t in ["Read", "Read", "Grep", "LS", "Bash", "Bash", "WebFetch", "mcp__github__create_issue", "Edit"] {
            g.call(t);
        }
        g.edit("/p/src/parser.rs", 4, 12);
        assert_eq!(tool_group_row(&g, OFF, 120), "  ◇ Edited parser.rs, explored 1 directory, 2 files, 1 search, 1 fetch, ran 2 commands, called 1 tool +12 −4");
        let mut new = ToolGroup::default();
        new.call("Write");
        new.edit("notes.txt", 0, 3);
        new.call("Bash");
        new.failed();
        assert_eq!(tool_group_row(&new, OFF, 80), "  ◇ Created notes.txt, ran 1 command +3 · 1 failed");
        let on = tool_group_row(&new, ON, 80);
        assert!(on.contains("\x1b[1mCreated") && on.contains("\x1b[32m+3") && on.contains("\x1b[31m· 1 failed"), "{on:?}");
        let mut reads = ToolGroup::default();
        reads.call("Read");
        assert_eq!(tool_group_row(&reads, OFF, 80), "  ◇ Explored 1 file");
        let mut many = ToolGroup::default();
        for f in ["a.rs", "b.rs", "c.rs"] {
            many.call("Edit");
            many.edit(f, 1, 1);
        }
        assert_eq!(tool_group_row(&many, OFF, 80), "  ◇ Edited 3 files +3 −3");
        assert!(crate::tui::render::display_width(&tool_group_row(&g, OFF, 30)) <= 30);
        assert!(tool_failed("Bash", true, "test result: FAILED. 1 failed") && tool_failed("Read", false, "") && !tool_failed("Read", true, "FAILED"));
    }
