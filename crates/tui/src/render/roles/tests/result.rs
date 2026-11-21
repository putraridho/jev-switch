use super::*;

    #[test]
    fn a_failed_call_under_the_run_says_what_and_how_it_failed() {
        let out = "running 4 tests\ntest empty ... FAILED\n\ntest result: FAILED. 3 passed; 1 failed";
        let rows = tool_failure_rows("Bash cargo test parser", true, out, Path::new("/p"), OFF, 80);
        assert_eq!(rows, vec!["    ✗ Bash cargo test parser", "      test result: FAILED. 3 passed; 1 failed  (+2 lines)", "      running 4 tests", "      test empty ... FAILED"]);
        let rows = tool_failure_rows("Read /p/a.rs", false, "no such file", Path::new("/p"), ON, 80);
        assert!(rows[0].contains("\x1b[31m✗") && strip_ansi(&rows[0]) == "    ✗ Read a.rs" && rows.len() == 2, "{rows:?}");
    }


    #[test]
    fn results_say_what_the_person_wants_to_know() {
        let r = |tool: &str, ok: bool, text: &str| tool_result_row(tool, ok, text, OFF, 80);
        assert_eq!(r("Read", true, "1→a\n2→b\n3→c").as_deref(), Some("    ⎿ ✓ read 3 lines"));
        assert_eq!(r("Grep", true, "").as_deref(), Some("    ⎿ ✓ no matches"));
        assert_eq!(r("Glob", true, "a\nb").as_deref(), Some("    ⎿ ✓ 2 paths"));
        assert_eq!(r("Bash", true, "running 3 tests\n\ntest result: ok. 3 passed").as_deref(), Some("    ⎿ ✓ test result: ok. 3 passed  (+1 lines)"));
        assert_eq!(r("shell", true, "").as_deref(), Some("    ⎿ ✓ done, no output"));
        assert_eq!(r("Bash", false, "error: no such file\nmore").as_deref(), Some("    ⎿ ✗ error: no such file"));
        assert_eq!(r("Edit", true, "The file was updated"), None, "the diff above says it");
        let failing = tool_result_row("Bash", true, "test empty ... FAILED\ntest result: FAILED. 3 passed; 1 failed", ON, 80).unwrap_or_default();
        assert!(failing.contains("\x1b[33mtest result: FAILED"), "a failing test run is yellow, not a quiet dim line: {failing:?}");
        assert!(tool_result_row("Bash", false, "boom", ON, 80).is_some_and(|s| s.contains("\x1b[31mboom")));
    }


    #[test]
    fn failing_output_is_folded_under_the_result_row_and_quiet_output_is_not() {
        let long = (0..9).map(|i| format!("error line {i}")).collect::<Vec<_>>().join("\n");
        let out = tool_output_fold("Bash", false, &long, OFF, 80).unwrap_or_default();
        let rows: Vec<&str> = out.lines().collect();
        assert_eq!(rows.len(), FOLD_KEEP + 1, "{out}");
        assert_eq!(rows[0], "      error line 1", "line 0 is on the result row above");
        assert_eq!(rows[FOLD_KEEP], "      … +4 lines (output cut)");
        assert!(tool_output_fold("Bash", true, "a\nb\nc", OFF, 80).is_none());
        assert!(tool_output_fold("Bash", false, "one line", OFF, 80).is_none());
    }
