//! Highlighter tests: language names and text that never changes.

use super::*;

#[test]
fn languages_are_found_by_name() {
    for (name, lang) in [
        ("rust", Lang::Rust),
        ("RS", Lang::Rust),
        ("py", Lang::Python),
        ("tsx", Lang::JavaScript),
        ("go", Lang::Go),
        ("zsh", Lang::Shell),
        ("json", Lang::Json),
        ("toml", Lang::Toml),
        ("yml", Lang::Yaml),
        ("cpp", Lang::C),
        ("java", Lang::Java),
        ("sql", Lang::Sql),
        ("diff", Lang::Diff),
    ] {
        assert_eq!(Lang::from_info(name), Some(lang), "{name}");
    }
    assert_eq!(Lang::from_info("brainfuck"), None);
    assert_eq!(Lang::from_info(""), None);
}

#[test]
fn off_style_gives_the_line_back_without_escapes() {
    let samples = [
        (Lang::Rust, "fn main() { let s = \"hi\"; // note"),
        (Lang::Python, "def f(x): return 'a' # c"),
        (Lang::Diff, "+added"),
        (Lang::Sql, "SELECT 1 -- one"),
        (Lang::Json, "{\"a\": [1, 2.5, null]}"),
    ];
    for (lang, line) in samples {
        let out = hl(lang, line, OFF);
        no_esc(&out);
        assert_eq!(out, line);
    }
}

#[test]
fn colouring_never_changes_the_text() {
    let samples = [
        (Lang::Rust, "pub fn x<'a>(s: &'a str) -> u32 { 0x1F + 1_000 } /* c */ // d"),
        (Lang::JavaScript, "const t = `a ${b}`; // x"),
        (Lang::Shell, "echo \"$#\" # hi"),
        (Lang::Yaml, "key: \"v\" # c"),
        (Lang::C, "#include <stdio.h> /* unclosed"),
        (Lang::Go, "s := \"unclosed \\"),
    ];
    for (lang, line) in samples {
        assert_eq!(strip_ansi(&hl(lang, line, ON)), line);
    }
}

#[test]
fn odd_lines_do_not_panic() {
    for lang in [Lang::Rust, Lang::Python, Lang::JavaScript, Lang::Go, Lang::Shell, Lang::Json, Lang::Toml, Lang::Yaml, Lang::C, Lang::Java, Lang::Sql, Lang::Diff] {
        for line in ["", "\"", "\\", "\"\\", "/*", "*/", "/", "#", "-", "日本語 \"語\" 1", "1.", "1..2", "'", "`"] {
            let mut state = LineState::Normal;
            assert_eq!(strip_ansi(&highlight_line(lang, line, &mut state, ON)), line);
        }
    }
}
