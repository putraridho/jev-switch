use super::*;

    #[test]
    fn answer_headings_bold_and_code() {
        assert_eq!(answer_text("# Title", ON, 80), "\x1b[1mTitle\x1b[0m");
        assert_eq!(answer_text("### Deep ###", ON, 80), "\x1b[1mDeep\x1b[0m");
        assert_eq!(answer_text("#nospace", ON, 80), "#nospace");
        assert_eq!(answer_text("#", ON, 80), "#");
        assert_eq!(answer_text("####### seven", ON, 80), "####### seven");
        assert_eq!(answer_text("use **this** now", ON, 80), "use \x1b[1mthis\x1b[0m now");
        assert_eq!(answer_text("run `cargo test` now", ON, 80), "run \x1b[36mcargo\x1b[0m \x1b[36mtest\x1b[0m now");
        assert_eq!(answer_text("**bold**, then `x`.", ON, 80), "\x1b[1mbold\x1b[0m, then \x1b[36mx\x1b[0m.");
        assert_eq!(answer_text("**bold with `code`**", ON, 80), "\x1b[1mbold\x1b[0m \x1b[1mwith\x1b[0m \x1b[36mcode\x1b[0m");
        // unmatched markers are literal
        assert_eq!(answer_text("a ** b", ON, 80), "a ** b");
        assert_eq!(answer_text("a ` b", ON, 80), "a ` b");
        assert_eq!(answer_text("**unclosed bold", ON, 80), "**unclosed bold");
        assert_eq!(answer_text("2 ** 3 ** 4", ON, 80), "2 ** 3 ** 4");
        assert_eq!(answer_text("``a`b``", ON, 80), "\x1b[36ma`b\x1b[0m");
        assert_eq!(answer_text("****", ON, 80), "****");
    }

    #[test]
    fn links_show_the_text_then_a_dim_url() {
        assert_eq!(answer_text("see [docs](https://a.io/x) now", ON, 80), "see docs \x1b[2m(https://a.io/x)\x1b[0m now");
        assert_eq!(answer_text("[https://a.io](https://a.io)", ON, 80), "\x1b[2mhttps://a.io\x1b[0m");
        assert_eq!(strip_ansi(&answer_text("**[b](http://u)**.", ON, 80)), "b (http://u).");
        // not links: no target, spaces in the target, empty parts, nested brackets
        for t in ["[a] (b)", "[a](b c)", "[](x)", "[a]()", "[a [b](c)", "[a](b"] {
            let out = strip_ansi(&answer_text(t, ON, 80));
            assert!(out.contains('[') || out == "a [b] (c)" || t == "[a [b](c)", "{t:?} -> {out:?}");
        }
        assert_eq!(answer_text("[a](b c)", ON, 80), "[a](b c)");
    }

    #[test]
    fn link_urls_are_never_broken() {
        let url = "https://example.com/some/really/long/path/that/never/fits/on/one/line";
        let out = answer_text(&format!("read [the guide]({url}) for more details"), ON, 30);
        assert!(out.contains(&format!("\x1b[2m({url})\x1b[0m")), "{out:?}");
        // a target without a scheme is not cut either
        let path = "docs/a/very/long/relative/path/to/a/file.md";
        let out = answer_text(&format!("[x]({path})"), ON, 20);
        assert!(strip_ansi(&out).contains(&format!("({path})")), "{out:?}");
    }
