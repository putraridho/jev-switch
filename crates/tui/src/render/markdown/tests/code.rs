use super::*;

    #[test]
    fn answer_fenced_code() {
        let t = "before\n```text\nfn main() {\n\n    let x = 1;\n}\n```\nafter";
        assert_eq!(
            answer_text(t, ON, 80),
            "before\n  \x1b[2mtext\x1b[0m\n    \x1b[2mfn main() {\x1b[0m\n\n    \x1b[2m    let x = 1;\x1b[0m\n    \x1b[2m}\x1b[0m\nafter"
        );
        // no language, tilde fence, longer closing fence
        assert_eq!(answer_text("~~~\ncode\n~~~~", ON, 80), "    \x1b[2mcode\x1b[0m");
        // markdown inside code is not interpreted and not wrapped
        let long = format!("```\n# not a heading **x** {}\n```", "word ".repeat(40));
        let out = answer_text(&long, ON, 30);
        assert!(strip_ansi(&out).contains("# not a heading **x**"));
        assert_eq!(out.matches('\n').count(), 0, "one code line, not wrapped");
        // a fence shorter than the opening one does not close it
        assert_eq!(answer_text("````\n```\nstill code\n````\nprose", ON, 80), "    \x1b[2m```\x1b[0m\n    \x1b[2mstill code\x1b[0m\nprose");
        // a closing fence cannot carry an info string
        assert_eq!(answer_text("```\n```rust\n```", ON, 80), "    \x1b[2m```rust\x1b[0m");
        // triple backticks inline are not a fence
        assert_eq!(answer_text("use ```x``` here", ON, 80), "use \x1b[36mx\x1b[0m here");
    }

    #[test]
    fn known_languages_are_highlighted_and_keep_their_layout() {
        let t = "```rust\n    let s = \"hi\";\n\nfn f() {}\n```";
        let out = answer_text(t, ON, 80);
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(strip_ansi(&out), "  rust\n        let s = \"hi\";\n\n    fn f() {}");
        assert!(lines[1].contains("\x1b[35mlet\x1b[0m") && lines[1].contains("\x1b[32m\"hi\"\x1b[0m"), "{out:?}");
        assert!(lines[3].contains("\x1b[35mfn\x1b[0m"), "{out:?}");
        // a long code line is still not wrapped
        let long = format!("```rust\nlet x = \"{}\";\n```", "word ".repeat(40));
        assert_eq!(answer_text(&long, ON, 30).matches('\n').count(), 1);
    }

    #[test]
    fn diff_blocks_colour_added_and_removed_lines() {
        let out = answer_text("```diff\n@@ -1 +1 @@\n-old\n+new\n ctx\n```", ON, 80);
        assert!(out.contains("\x1b[36m@@ -1 +1 @@\x1b[0m"), "{out:?}");
        assert!(out.contains("    \x1b[31m-old\x1b[0m"), "{out:?}");
        assert!(out.contains("    \x1b[32m+new\x1b[0m"), "{out:?}");
    }

    #[test]
    fn highlighted_blocks_have_no_escapes_with_style_off() {
        let t = "```rust\nfn main() { let s = \"x\"; }\n```\n```diff\n+a\n-b\n```\n| a | b |\n|---|---|\n| 1 | 2 |\n[t](http://u)";
        assert_eq!(answer_text(t, OFF, 80), t);
        let tty_plain = Style::resolve(true, Some("1"), None, None);
        no_esc(&answer_text(t, tty_plain, 80));
    }

    #[test]
    fn answer_unclosed_fence_swallows_the_rest_as_code() {
        let out = answer_text("```txt\nprint(1)\n# still code\n**not bold**", ON, 80);
        assert_eq!(
            out,
            "  \x1b[2mtxt\x1b[0m\n    \x1b[2mprint(1)\x1b[0m\n    \x1b[2m# still code\x1b[0m\n    \x1b[2m**not bold**\x1b[0m"
        );
        assert_eq!(answer_text("```", ON, 80), "");
    }
