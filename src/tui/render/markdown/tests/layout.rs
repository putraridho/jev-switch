use super::*;

    #[test]
    fn answer_lists_keep_markers_and_hang() {
        assert_eq!(answer_text("- one\n* two\n+ three", ON, 80), "- one\n* two\n+ three");
        assert_eq!(answer_text("1. first\n12) second", ON, 80), "1. first\n12) second");
        assert_eq!(answer_text("  - nested **b**", ON, 80), "  - nested \x1b[1mb\x1b[0m");
        let out = answer_text("- alpha beta gamma delta epsilon zeta", ON, 20);
        assert_eq!(out, "- alpha beta gamma\n  delta epsilon zeta");
        let out = answer_text("10. alpha beta gamma delta", ON, 20);
        assert_eq!(out, "10. alpha beta gamma\n    delta");
        // a bold line starting with ** is not a bullet
        assert_eq!(answer_text("**Note** here", ON, 80), "\x1b[1mNote\x1b[0m here");
        // "-5 degrees" is prose
        assert_eq!(answer_text("-5 degrees", ON, 80), "-5 degrees");
        // quotes keep their marker
        assert_eq!(answer_text("> quoted text", ON, 80), "> quoted text");
    }

    #[test]
    fn answer_wraps_prose_not_urls_tables_or_blank_lines() {
        let url = "https://example.com/some/really/long/path/that/never/fits/on/one/line";
        let out = answer_text(&format!("see {url} for more details on this"), ON, 30);
        assert!(out.contains(url));
        assert!(out.split('\n').any(|l| l == url));
        let table = "| a | b |\n|---|---|\n| a very long cell that would wrap if it were prose | x |";
        assert_eq!(answer_text(table, ON, 20), table);
        assert_eq!(answer_text("a\n\n\nb\n", ON, 80), "a\n\n\nb\n");
        assert_eq!(answer_text("one two three four five six", ON, 20), "one two three four\nfive six");
        assert_eq!(answer_text("a\r\nb\r\n", ON, 80), "a\nb\n");
        // two-line paragraphs stay two lines (the model chose those breaks)
        assert_eq!(answer_text("first line\nsecond line", ON, 80), "first line\nsecond line");
        // leading indentation of prose is kept, but never leaves no room
        assert_eq!(answer_text("  indented text", ON, 80), "  indented text");
        let deep = format!("{}text that is long enough to wrap around", " ".repeat(30));
        for l in answer_text(&deep, ON, 20).split('\n') {
            assert!(display_width(l) <= 20, "{l:?}");
        }
    }

    #[test]
    fn answer_styled_spans_survive_wrapping() {
        let out = answer_text("**alpha beta gamma delta** epsilon zeta", ON, 20);
        assert_eq!(strip_ansi(&out), "alpha beta gamma\ndelta epsilon zeta");
        // each word opens and closes its own bold, so a wrap never leaks a style
        assert_eq!(out.split('\n').next().unwrap(), "\x1b[1malpha\x1b[0m \x1b[1mbeta\x1b[0m \x1b[1mgamma\x1b[0m");
    }

    #[test]
    fn answer_prose_never_exceeds_the_width() {
        let samples = [
            "The quick brown fox jumps over the lazy dog and keeps running through the forest.",
            "日本語のテキストはスペースがないのでとても長い一つの単語として扱われます。これは長い。",
            "🚀🚀🚀 launch 🚀🚀 sequence 🚀 initiated 🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀🚀",
            "supercalifragilisticexpialidocious_and_then_some_more_characters_without_any_space",
            "**bold supercalifragilisticexpialidocious_and_then_some_more_characters** `inlinecode_that_is_quite_long_indeed`",
            "- a list item with `code` and **bold** and a verylongwordthatdoesnotfitanywhereatall_really",
            "e\u{301}e\u{301}e\u{301} combining marks e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}",
            "# A heading that is long enough that it has to wrap onto another row of text",
            "> a quote that is long enough that it has to wrap onto another row of text",
        ];
        for t in samples {
            for w in [MIN_WIDTH, 21, 25, 33, 40, 80] {
                for l in answer_text(t, ON, w).split('\n') {
                    assert!(display_width(l) <= w, "{l:?} at {w} for {t:?}");
                }
            }
        }
    }
