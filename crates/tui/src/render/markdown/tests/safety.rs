use super::*;

    #[test]
    fn answer_text_is_untouched_without_style() {
        for t in [
            "# Title\n**bold** and `code`\n```rust\nfn main() {}\n```\n",
            "",
            "unclosed ```\n\x1b[31mraw\x1b[0m escapes stay in logs",
            "日本語 🚀 e\u{301}",
            &"word ".repeat(200),
        ] {
            assert_eq!(answer_text(t, OFF, 20), t);
        }
    }

    #[test]
    fn answer_text_survives_weird_input() {
        let weird = [
            "",
            "\n\n\n",
            "```",
            "``````",
            "`",
            "``",
            "`a",
            "**",
            "*",
            "* ",
            "- ",
            "1.",
            "1. ",
            "#",
            "# ",
            "| ",
            "\t\t- tabbed",
            "\x1b[31mred\x1b[0m text",
            "\x1b",
            "\u{0}\u{7}\u{8}",
            "🚀`🚀**🚀`**",
            "**`**`**",
            &"`".repeat(500),
            &"*".repeat(501),
            &"a".repeat(10_000),
            &"- ".repeat(1_000),
            "e\u{301}",
            "\u{200d}\u{200d}",
            "```rust\n```rust\n```\n```",
            "> > > > >",
            "99999. not a list",
        ];
        for t in weird {
            for w in [0, 1, 20, 80] {
                let out = answer_text(t, ON, w);
                // model text can never drive the terminal: only our own SGR sequences remain
                let mut rest = out.as_str();
                while let Some(i) = rest.find('\x1b') {
                    let after = &rest[i..];
                    let end = after.find('m').expect("only SGR sequences");
                    assert!(after[1..end].chars().all(|c| c == '[' || c.is_ascii_digit() || c == ';'), "{out:?}");
                    rest = &after[end + 1..];
                }
                let _ = answer_text(t, OFF, w);
            }
        }
    }

    #[test]
    fn answer_text_strips_model_escapes() {
        let out = answer_text("hello \x1b[2Jworld \x1b]0;title\x07done", ON, 80);
        assert_eq!(out, "hello world done");
        assert_eq!(answer_text("```\n\x1b[31mred\n```", ON, 80), "    \x1b[2mred\x1b[0m");
    }

    #[test]
    fn colour_off_on_a_terminal_still_strips_control_sequences_but_pipes_stay_raw() {
        let hostile = "ok\x1b]0;pwned\x07 \x1b[2Jdone\x07";
        let tty_no_colour = Style::resolve(true, Some("1"), None, None);
        assert!(!tty_no_colour.enabled && tty_no_colour.terminal);
        let shown = answer_text(hostile, tty_no_colour, 80);
        assert!(!shown.contains('\x1b') && !shown.contains('\x07'), "{shown:?}");
        assert!(shown.contains("ok") && shown.contains("done"));
        let piped = Style::resolve(false, None, None, None);
        assert_eq!(answer_text(hostile, piped, 80), hostile, "non-terminal output is passed through byte for byte");
    }
