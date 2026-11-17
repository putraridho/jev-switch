//! Small hand-rolled syntax colouring for fenced code blocks. It is a line scanner, not
//! a parser: it finds keywords, strings, comments and numbers, and leaves the rest
//! plain. The text of a line is never changed, only wrapped in colour, so a disabled
//! [`Style`] gives back exactly the line it was given.

use super::style::Style;

/// A language the highlighter knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lang {
    Rust,
    Python,
    JavaScript,
    Go,
    Shell,
    Json,
    Toml,
    Yaml,
    C,
    Java,
    Sql,
    Diff,
}

impl Lang {
    /// The language named by the info string of a fence (`rust`, `py`, `tsx`...), if known.
    pub(super) fn from_info(info: &str) -> Option<Lang> {
        let lang = match info.to_ascii_lowercase().as_str() {
            "rust" | "rs" => Lang::Rust,
            "python" | "py" => Lang::Python,
            "js" | "javascript" | "jsx" | "mjs" | "cjs" | "ts" | "typescript" | "tsx" => Lang::JavaScript,
            "go" | "golang" => Lang::Go,
            "sh" | "bash" | "zsh" | "shell" => Lang::Shell,
            "json" | "jsonc" => Lang::Json,
            "toml" => Lang::Toml,
            "yaml" | "yml" => Lang::Yaml,
            "c" | "h" | "cpp" | "c++" | "cc" | "hpp" => Lang::C,
            "java" => Lang::Java,
            "sql" => Lang::Sql,
            "diff" | "patch" => Lang::Diff,
            _ => return None,
        };
        Some(lang)
    }
}

/// What a line scanner carries from one line of a block to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum LineState {
    #[default]
    Normal,
    InBlockComment,
}

/// What a run of code is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    Keyword,
    Str,
    Comment,
    Number,
}

/// How to scan one language.
struct Spec {
    keywords: &'static [&'static str],
    line_comments: &'static [&'static str],
    block_comments: bool,
    quotes: &'static [char],
    ignore_case: bool,
}

const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let",
    "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
    "use", "where", "while",
];
const PYTHON_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
    "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "False", "try", "while", "with", "yield",
];
const JS_KW: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "else", "export", "extends", "false", "finally", "for",
    "function", "if", "import", "in", "instanceof", "interface", "let", "new", "null", "of", "return", "switch", "this", "throw", "true", "try", "type",
    "typeof", "undefined", "var", "while", "yield",
];
const GO_KW: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "false", "for", "func", "go", "if", "import", "interface", "map", "nil",
    "package", "range", "return", "select", "struct", "switch", "true", "type", "var",
];
const SHELL_KW: &[&str] = &[
    "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if", "in", "local", "return", "then", "until", "while",
];
const JSON_KW: &[&str] = &["true", "false", "null"];
const TOML_KW: &[&str] = &["true", "false"];
const YAML_KW: &[&str] = &["true", "false", "null", "yes", "no"];
const C_KW: &[&str] = &[
    "auto", "bool", "break", "case", "char", "class", "const", "continue", "default", "do", "double", "else", "enum", "extern", "false", "float", "for",
    "if", "inline", "int", "long", "namespace", "new", "nullptr", "return", "short", "signed", "sizeof", "static", "struct", "switch", "template",
    "true", "typedef", "union", "unsigned", "using", "void", "while",
];
const JAVA_KW: &[&str] = &[
    "abstract", "boolean", "break", "case", "catch", "class", "continue", "default", "else", "enum", "extends", "false", "final", "finally", "for", "if",
    "implements", "import", "instanceof", "int", "interface", "new", "null", "package", "private", "protected", "public", "return", "static", "super",
    "switch", "this", "throw", "throws", "true", "try", "void", "while",
];
const SQL_KW: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create", "table", "drop", "alter", "join", "left", "right",
    "inner", "outer", "on", "group", "by", "order", "having", "limit", "and", "or", "not", "null", "as", "distinct", "union", "index", "primary", "key",
];

const SLASH: &[&str] = &["//"];
const HASH: &[&str] = &["#"];
const DASH: &[&str] = &["--"];
const NONE: &[&str] = &[];
const DOUBLE: &[char] = &['"'];
const BOTH: &[char] = &['"', '\''];
const ALL: &[char] = &['"', '\'', '`'];

/// The scanning rules of `lang`. A diff has none: it is coloured by line.
fn spec_of(lang: Lang) -> Spec {
    let (keywords, line_comments, block_comments, quotes, ignore_case) = match lang {
        Lang::Rust => (RUST_KW, SLASH, true, DOUBLE, false),
        Lang::Python => (PYTHON_KW, HASH, false, BOTH, false),
        Lang::JavaScript => (JS_KW, SLASH, true, ALL, false),
        Lang::Go => (GO_KW, SLASH, true, ALL, false),
        Lang::Shell => (SHELL_KW, HASH, false, BOTH, false),
        Lang::Json => (JSON_KW, NONE, false, DOUBLE, false),
        Lang::Toml => (TOML_KW, HASH, false, BOTH, false),
        Lang::Yaml => (YAML_KW, HASH, false, BOTH, false),
        Lang::C => (C_KW, SLASH, true, BOTH, false),
        Lang::Java => (JAVA_KW, SLASH, true, BOTH, false),
        Lang::Sql => (SQL_KW, DASH, true, BOTH, true),
        Lang::Diff => (NONE, NONE, false, DOUBLE, false),
    };
    Spec { keywords, line_comments, block_comments, quotes, ignore_case }
}

/// Where a scan is: the characters of the line and what has been found so far.
struct Scanner<'a> {
    chars: Vec<char>,
    spec: &'a Spec,
    out: Vec<(Kind, String)>,
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// End of a block comment that continues from `from`, and whether it closed on this line.
fn block_comment_end(chars: &[char], from: usize) -> (usize, bool) {
    match (from..chars.len().saturating_sub(1)).find(|&j| chars[j] == '*' && chars[j + 1] == '/') {
        Some(j) => (j + 2, true),
        None => (chars.len(), false),
    }
}

/// End of the string that opens with `quote` at `start`; a backslash escapes the next
/// character, and an unclosed string runs to the end of the line.
fn string_end(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
        } else if chars[i] == quote {
            return i + 1;
        } else {
            i += 1;
        }
    }
    chars.len()
}

/// End of the number that starts at `start` (digits, letters, `_`, and `.` before a digit).
fn number_end(chars: &[char], start: usize) -> usize {
    let mut i = start;
    while i < chars.len() && (is_ident_char(chars[i]) || (chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))) {
        i += 1;
    }
    i
}

fn ident_end(chars: &[char], start: usize) -> usize {
    start + chars[start..].iter().take_while(|c| is_ident_char(**c)).count()
}

impl Scanner<'_> {
    fn push(&mut self, kind: Kind, from: usize, to: usize) {
        let text: String = self.chars[from..to].iter().collect();
        match self.out.last_mut() {
            Some((k, t)) if *k == kind => t.push_str(&text),
            _ => self.out.push((kind, text)),
        }
    }

    fn starts_with(&self, at: usize, pat: &str) -> bool {
        pat.chars().enumerate().all(|(k, p)| self.chars.get(at + k) == Some(&p))
    }

    /// Whether a line comment starts at `at`. A `#` only counts at the start of a word,
    /// so `$#` and `a#b` are not comments.
    fn line_comment_at(&self, at: usize) -> bool {
        self.spec.line_comments.iter().any(|p| {
            self.starts_with(at, p) && (*p != "#" || at == 0 || !(is_ident_char(self.chars[at - 1]) || self.chars[at - 1] == '$'))
        })
    }

    fn is_keyword(&self, word: &str) -> bool {
        if self.spec.ignore_case {
            self.spec.keywords.contains(&word.to_ascii_lowercase().as_str())
        } else {
            self.spec.keywords.contains(&word)
        }
    }

    /// Scan one token at `i` and return where the next one starts.
    fn step(&mut self, i: usize, state: &mut LineState) -> usize {
        let c = self.chars[i];
        if self.line_comment_at(i) {
            self.push(Kind::Comment, i, self.chars.len());
            return self.chars.len();
        }
        if self.spec.block_comments && self.starts_with(i, "/*") {
            return self.block_comment(i + 2, i, state);
        }
        if self.spec.quotes.contains(&c) {
            let end = string_end(&self.chars, i, c);
            self.push(Kind::Str, i, end);
            return end;
        }
        if c.is_ascii_digit() {
            let end = number_end(&self.chars, i);
            self.push(Kind::Number, i, end);
            return end;
        }
        if is_ident_char(c) {
            let end = ident_end(&self.chars, i);
            let word: String = self.chars[i..end].iter().collect();
            self.push(if self.is_keyword(&word) { Kind::Keyword } else { Kind::Plain }, i, end);
            return end;
        }
        self.push(Kind::Plain, i, i + 1);
        i + 1
    }

    /// Colour a block comment that began at `from`, searching for its end after `search`.
    fn block_comment(&mut self, search: usize, from: usize, state: &mut LineState) -> usize {
        let (end, closed) = block_comment_end(&self.chars, search);
        self.push(Kind::Comment, from, end);
        *state = if closed { LineState::Normal } else { LineState::InBlockComment };
        end
    }

    fn run(&mut self, state: &mut LineState) {
        let mut i = 0;
        if *state == LineState::InBlockComment {
            i = self.block_comment(0, 0, state);
        }
        while i < self.chars.len() {
            i = self.step(i, state);
        }
    }
}

/// `(kind, text)` runs of `line`, and the state to carry to the next line.
fn tokenize(spec: &Spec, line: &str, state: LineState) -> (Vec<(Kind, String)>, LineState) {
    let mut scanner = Scanner { chars: line.chars().collect(), spec, out: Vec::new() };
    let mut next = state;
    scanner.run(&mut next);
    (scanner.out, next)
}

fn paint(style: Style, kind: Kind, text: &str) -> String {
    match kind {
        Kind::Plain => text.to_string(),
        Kind::Keyword => style.magenta(text),
        Kind::Str => style.green(text),
        Kind::Comment => style.dim(text),
        Kind::Number => style.yellow(text),
    }
}

/// A diff line: `+` green, `-` red, `@@` cyan, anything else dim.
fn diff_line(line: &str, style: Style) -> String {
    if line.starts_with('+') {
        style.green(line)
    } else if line.starts_with('-') {
        style.red(line)
    } else if line.starts_with("@@") {
        style.cyan(line)
    } else {
        style.dim(line)
    }
}

/// One line of a code block in `lang`, coloured. `state` carries an open block comment
/// to the next line. The text of the line is not changed.
pub(super) fn highlight_line(lang: Lang, line: &str, state: &mut LineState, style: Style) -> String {
    if lang == Lang::Diff {
        return diff_line(line, style);
    }
    let (runs, next) = tokenize(&spec_of(lang), line, *state);
    *state = next;
    runs.iter().map(|(kind, text)| paint(style, *kind, text)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::strip_ansi;
    use crate::uifmt::testutil::*;

    fn hl(lang: Lang, line: &str, style: Style) -> String {
        highlight_line(lang, line, &mut LineState::Normal, style)
    }

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
    fn rust_colours_keywords_and_strings() {
        let out = hl(Lang::Rust, "fn main() { let s = \"hi\"; }", ON);
        assert!(out.contains("\x1b[35mfn\x1b[0m"), "{out:?}");
        assert!(out.contains("\x1b[35mlet\x1b[0m"), "{out:?}");
        assert!(out.contains("\x1b[32m\"hi\"\x1b[0m"), "{out:?}");
        assert!(!out.contains("\x1b[35mmain"), "identifiers are not keywords");
    }

    #[test]
    fn numbers_and_comments_are_coloured() {
        let out = hl(Lang::Rust, "let x = 42; // answer", ON);
        assert!(out.contains("\x1b[33m42\x1b[0m"), "{out:?}");
        assert!(out.contains("\x1b[2m// answer\x1b[0m"), "{out:?}");
        // a digit inside a name is part of the name
        assert!(!hl(Lang::Rust, "x1", ON).contains('\x1b'));
    }

    #[test]
    fn a_comment_marker_inside_a_string_is_text() {
        let out = hl(Lang::Rust, "let u = \"http://x\";", ON);
        assert!(out.contains("\x1b[32m\"http://x\"\x1b[0m"), "{out:?}");
        assert!(!out.contains("\x1b[2m"));
    }

    #[test]
    fn escaped_quotes_stay_inside_the_string() {
        let out = hl(Lang::Python, r#"x = "a\"b" + 1"#, ON);
        assert!(out.contains(r#"[32m"a\"b"[0m"#.replace('[', "\x1b[").as_str()), "{out:?}");
    }

    #[test]
    fn hash_comments_need_a_word_start() {
        assert!(hl(Lang::Shell, "ls # list", ON).contains("\x1b[2m# list\x1b[0m"));
        assert!(!hl(Lang::Shell, "echo $#", ON).contains("\x1b[2m"));
        assert!(hl(Lang::Python, "# top", ON).contains("\x1b[2m# top\x1b[0m"));
    }

    #[test]
    fn block_comments_carry_across_lines() {
        let mut state = LineState::Normal;
        let first = highlight_line(Lang::C, "int a; /* start", &mut state, ON);
        assert_eq!(state, LineState::InBlockComment);
        assert!(first.contains("\x1b[2m/* start\x1b[0m"), "{first:?}");
        let mid = highlight_line(Lang::C, "still comment", &mut state, ON);
        assert_eq!(mid, "\x1b[2mstill comment\x1b[0m");
        let last = highlight_line(Lang::C, "end */ return 1;", &mut state, ON);
        assert_eq!(state, LineState::Normal);
        assert!(last.starts_with("\x1b[2mend */\x1b[0m"), "{last:?}");
        assert!(last.contains("\x1b[35mreturn\x1b[0m"), "{last:?}");
    }

    #[test]
    fn sql_keywords_ignore_case_and_use_dash_comments() {
        let out = hl(Lang::Sql, "Select name FROM t -- all", ON);
        assert!(out.contains("\x1b[35mSelect\x1b[0m"), "{out:?}");
        assert!(out.contains("\x1b[35mFROM\x1b[0m"), "{out:?}");
        assert!(out.contains("\x1b[2m-- all\x1b[0m"), "{out:?}");
    }

    #[test]
    fn json_colours_strings_numbers_and_literals() {
        let out = hl(Lang::Json, "{\"a\": [1, null]}", ON);
        assert!(out.contains("\x1b[32m\"a\"\x1b[0m"), "{out:?}");
        assert!(out.contains("\x1b[33m1\x1b[0m"), "{out:?}");
        assert!(out.contains("\x1b[35mnull\x1b[0m"), "{out:?}");
    }

    #[test]
    fn diff_colours_added_removed_and_hunk_lines() {
        assert_eq!(hl(Lang::Diff, "+added", ON), "\x1b[32m+added\x1b[0m");
        assert_eq!(hl(Lang::Diff, "-removed", ON), "\x1b[31m-removed\x1b[0m");
        assert_eq!(hl(Lang::Diff, "@@ -1,2 +1,2 @@", ON), "\x1b[36m@@ -1,2 +1,2 @@\x1b[0m");
        assert_eq!(hl(Lang::Diff, " context", ON), "\x1b[2m context\x1b[0m");
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
}
