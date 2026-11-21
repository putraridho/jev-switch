//! Small hand-rolled syntax colouring for fenced code blocks. It is a line scanner, not
//! a parser: it finds keywords, strings, comments and numbers, and leaves the rest
//! plain. The text of a line is never changed, only wrapped in colour, so a disabled
//! [`Style`] gives back exactly the line it was given.

use crate::render::style::Style;
use scanner::tokenize;
use spec::spec_of;

mod scanner;
mod spec;
#[cfg(test)]
mod tests;


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
