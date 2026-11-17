//! Tab completion and the inline hint for the slash commands.

use super::commands::{Cmd, COMMANDS};

/// What the line editor knows when completing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompleteCtx {
    /// Providers that are ready (`codex`, `claude`).
    pub ready_providers: Vec<String>,
    /// Usable models as `provider:model`.
    pub models: Vec<String>,
    /// Paths changed in this session, for `/diff`.
    pub changed_files: Vec<String>,
}

/// One completion. `replacement` replaces the word being completed (the text from
/// [`completion_start`] to the cursor); `display` is what a menu shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub replacement: String,
    pub display: String,
}

/// Byte offset where the word under the cursor (the end of `line`) starts.
pub fn completion_start(line: &str) -> usize {
    line.rfind(char::is_whitespace).map_or(0, |i| i + line[i..].chars().next().map_or(1, char::len_utf8))
}

fn starts_with_ci(word: &str, prefix: &str) -> bool {
    word.to_lowercase().starts_with(&prefix.to_lowercase())
}

fn commands_sorted() -> Vec<&'static Cmd> {
    let mut v: Vec<&Cmd> = COMMANDS.iter().collect();
    v.sort_by_key(|c| c.name);
    v
}

/// The words an argument of `command` may be.
fn arg_choices(command: &str, ctx: &CompleteCtx) -> Vec<String> {
    let words: Vec<&str> = match command {
        "/provider" => vec!["auto", "off"],
        "/orchestrate" => vec!["on", "off"],
        "/providers" => vec!["refresh"],
        "/permissions" => vec!["allow", "remove", "dir", "list"],
        "/model" => ["auto"]
            .into_iter()
            .chain(ctx.ready_providers.iter().map(String::as_str))
            .chain(ctx.models.iter().map(String::as_str))
            .chain(["sonnet", "opus", "haiku"])
            .collect(),
        "/diff" => ctx.changed_files.iter().map(String::as_str).filter(|f| !f.contains(char::is_whitespace)).collect(),
        _ => Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for w in words {
        if !out.iter().any(|o| o == w) {
            out.push(w.to_string());
        }
    }
    out
}

/// Candidates for the word under the cursor (the end of `line`): slash commands for the
/// first word; for the first argument of a command, its choices: `/model` takes `auto`,
/// each ready provider, `provider:model`, and the Claude aliases; `/provider` `auto|off`;
/// `/orchestrate` `on|off`; `/providers` `refresh`; `/diff` the changed files. Matching
/// ignores case. Prose (a line not starting with `/`) completes nothing.
pub fn complete(line: &str, ctx: &CompleteCtx) -> Vec<Candidate> {
    if !line.starts_with('/') {
        return Vec::new();
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let typing_next = line.ends_with(char::is_whitespace);
    if words.len() == 1 && !typing_next {
        let col = COMMANDS.iter().map(|c| c.name.len()).max().unwrap_or(0) + 2;
        return commands_sorted()
            .into_iter()
            .filter(|c| starts_with_ci(c.name, words[0]))
            .map(|c| Candidate { replacement: c.name.to_string(), display: format!("{:<col$}{}", c.name, c.about) })
            .collect();
    }
    let (arg_index, prefix) = if typing_next { (words.len() - 1, "") } else { (words.len() - 2, words[words.len() - 1]) };
    if arg_index != 0 {
        return Vec::new();
    }
    arg_choices(words[0], ctx)
        .into_iter()
        .filter(|w| starts_with_ci(w, prefix))
        .map(|w| Candidate { display: w.clone(), replacement: w })
        .collect()
}

/// The faint inline hint after the cursor, as plain text (the editor paints it dim): the
/// rest of the command when a prefix of one is typed (`/mod` → `el`), and the arguments
/// a complete command takes (`/model ` → `auto|codex|claude:...`; `/model` without the
/// space → ` auto|codex|claude:...`). Nothing once an argument is being typed.
pub fn hint(line: &str) -> Option<String> {
    if !line.starts_with('/') {
        return None;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let typing_next = line.ends_with(char::is_whitespace);
    let cmds = commands_sorted();
    let args_hint = |c: &Cmd| {
        (!c.hint.is_empty()).then(|| if typing_next { c.hint.to_string() } else { format!(" {}", c.hint) })
    };
    match (words.as_slice(), typing_next) {
        ([w], false) => match cmds.iter().find(|c| c.name == *w) {
            Some(c) => args_hint(c),
            None => cmds.iter().find(|c| c.name.starts_with(w)).map(|c| c.name[w.len()..].to_string()),
        },
        ([w], true) => cmds.iter().find(|c| c.name == *w).and_then(|c| args_hint(c)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::testutil::*;

    fn ctx() -> CompleteCtx {
        CompleteCtx {
            ready_providers: vec!["claude".into(), "codex".into()],
            models: vec!["claude:claude-sonnet-5-5".into(), "codex:gpt-6.1-sol".into(), "codex:gpt-6-luna".into()],
            changed_files: vec!["src/a.rs".into(), "src/b.rs".into(), "README.md".into(), "has space.txt".into()],
        }
    }

    fn replacements(line: &str) -> Vec<String> {
        complete(line, &ctx()).into_iter().map(|c| c.replacement).collect()
    }

    #[test]
    fn completes_slash_commands() {
        assert_eq!(replacements("/"), vec!["/diff", "/doctor", "/help", "/model", "/new", "/orchestrate", "/permissions", "/provider", "/providers", "/quit", "/resume", "/status", "/undo"]);
        assert_eq!(replacements("/mo"), vec!["/model"]);
        assert_eq!(replacements("/prov"), vec!["/provider", "/providers"]);
        assert_eq!(replacements("/MO"), vec!["/model"]);
        assert_eq!(replacements("/nope"), Vec::<String>::new());
        // the menu line names the command and says what it does
        let c = &complete("/mo", &ctx())[0];
        assert!(c.display.starts_with("/model") && c.display.contains("Pin a model"), "{}", c.display);
        // every command is a candidate for its own full name
        for cmd in &COMMANDS {
            assert!(replacements(cmd.name).contains(&cmd.name.to_string()));
        }
    }

    #[test]
    fn display_columns_align() {
        let cs = complete("/", &ctx());
        let col = cs[0].display.find(|c: char| c.is_uppercase()).unwrap();
        assert!(cs.iter().all(|c| c.display.find(|ch: char| ch.is_uppercase()) == Some(col)));
    }

    #[test]
    fn completes_arguments() {
        assert_eq!(
            replacements("/model "),
            vec!["auto", "claude", "codex", "claude:claude-sonnet-5-5", "codex:gpt-6.1-sol", "codex:gpt-6-luna", "sonnet", "opus", "haiku"]
        );
        assert_eq!(replacements("/model c"), vec!["claude", "codex", "claude:claude-sonnet-5-5", "codex:gpt-6.1-sol", "codex:gpt-6-luna"]);
        assert_eq!(replacements("/model codex:"), vec!["codex:gpt-6.1-sol", "codex:gpt-6-luna"]);
        assert_eq!(replacements("/model SO"), vec!["sonnet"]);
        assert_eq!(replacements("/model a"), vec!["auto"]);
        assert_eq!(replacements("/provider "), vec!["auto", "off"]);
        assert_eq!(replacements("/provider o"), vec!["off"]);
        assert_eq!(replacements("/orchestrate "), vec!["on", "off"]);
        assert_eq!(replacements("/orchestrate of"), vec!["off"]);
        assert_eq!(replacements("/providers r"), vec!["refresh"]);
        assert_eq!(replacements("/providers "), vec!["refresh"]);
        assert_eq!(replacements("/diff "), vec!["src/a.rs", "src/b.rs", "README.md"]);
        assert_eq!(replacements("/diff src/b"), vec!["src/b.rs"]);
        // a second argument, commands without arguments, prose and unknown commands complete nothing
        for none in ["/model auto ", "/model auto x", "/new ", "/help x", "fix the bug", "fix /model ", "", " /model ", "/nope "] {
            assert_eq!(replacements(none), Vec::<String>::new(), "{none:?}");
        }
        // duplicates collapse (a provider named like an alias)
        let dup = CompleteCtx { ready_providers: vec!["claude".into(), "claude".into()], ..Default::default() };
        let n = complete("/model cl", &dup).len();
        assert_eq!(n, 1);
        // an empty context still offers the fixed choices
        let empty: Vec<String> = complete("/model ", &CompleteCtx::default()).into_iter().map(|c| c.replacement).collect();
        assert_eq!(empty, vec!["auto", "sonnet", "opus", "haiku"]);
    }

    #[test]
    fn completion_start_is_the_word_start() {
        for (line, at) in [("", 0), ("/mo", 0), ("/model ", 7), ("/model co", 7), ("/model  co", 8), ("/diff src/a", 6), ("/model\tx", 7), ("日本 語", 7)] {
            assert_eq!(completion_start(line), at, "{line:?}");
        }
        // applying a candidate yields the full line
        let line = "/model co";
        let c = &complete(line, &ctx())[0];
        assert_eq!(format!("{}{}", &line[..completion_start(line)], c.replacement), "/model codex");
    }

    #[test]
    fn hints() {
        for (line, want) in [
            ("/model ", Some("auto|codex|claude:...")),
            ("/model", Some(" auto|codex|claude:...")),
            ("/provider ", Some("auto|off")),
            ("/orchestrate ", Some("on|off")),
            ("/providers ", Some("[refresh]")),
            ("/diff ", Some("[file]")),
            ("/mod", Some("el")),
            ("/prov", Some("ider")),
            ("/provider", Some(" auto|off")),
            ("/", Some("diff")),
            ("/new", None),
            ("/new ", None),
            ("/model co", None),
            ("/model auto ", None),
            ("/nope", None),
            ("hello", None),
            ("", None),
        ] {
            assert_eq!(hint(line).as_deref(), want, "{line:?}");
        }
        for line in ["/model ", "/mod", "/"] {
            no_esc(&hint(line).unwrap());
        }
    }
}
