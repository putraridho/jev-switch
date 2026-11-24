use super::*;

    #[test]
    fn thinking_shows_its_first_lines_behind_a_mark_and_counts_the_rest() {
        assert_eq!(thinking_block("one\n\ntwo", OFF, 80), "  ◆ Thought\n    one\n    two");
        let long = thinking_block("a\nb\nc\nd\ne", OFF, 80);
        assert_eq!(long, "  ◆ Thought\n    a\n    b\n    c\n    … 2 more lines of thinking");
        assert_eq!(thought_row("check the\ncallers", OFF, 80), "  ◆ Thought · check the callers");
        assert_eq!(thought_row("", OFF, 80), "  ◆ Thought");
        let long = "word ".repeat(40);
        let rows = thought_row(&long, OFF, 40);
        assert!(rows.lines().count() > 1, "a long thought wraps: {rows}");
        assert_eq!(rows.split_whitespace().filter(|w| *w == "word").count(), 40, "nothing is cut");
        assert!(thinking_block("a\nb\nc\nd", OFF, 80).ends_with("… 1 more line of thinking"));
    }


    #[test]
    fn narration_is_plain_bright_text_and_the_answer_is_set_apart() {
        assert_eq!(narration("Let me look.\n", OFF, 80), "  Let me look.");
        assert!(!narration("x", ON, 80).contains("\x1b["), "not dimmed: it is for reading");
        assert_eq!(answer_block("The answer.\n", 0), "\nThe answer.");
        assert_eq!(answer_block("One.\n\nTwo.\n", 2), "\n  One.\n\n  Two.");
        no_esc(&strip_ansi(&narration("a", ON, 80)));
    }
