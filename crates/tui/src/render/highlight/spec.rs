//! The scanning rules of each language: keyword tables and comment and quote syntax.

use super::Lang;

/// How to scan one language.
pub(super) struct Spec {
    pub(super) keywords: &'static [&'static str],
    pub(super) line_comments: &'static [&'static str],
    pub(super) block_comments: bool,
    pub(super) quotes: &'static [char],
    pub(super) ignore_case: bool,
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
pub(super) fn spec_of(lang: Lang) -> Spec {
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
