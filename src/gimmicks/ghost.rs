//! Ghost text: the dim rest of what you are typing, from your history and project files.
//! Tab (or Right at the end of the line) accepts it.

/// The text to show after `input` (not including it), or `None`. Looks at the word being
/// typed first: `@src/ma` completes to a project file; otherwise the newest history entry
/// that starts with the whole input wins (nothing for an empty input or an exact match).
pub fn ghost_suffix(input: &str, history_newest_first: &[String], files: &[String]) -> Option<String> {
    if input.trim().is_empty() {
        return None;
    }
    if let Some(word) = input.rsplit(' ').next().and_then(|w| w.strip_prefix('@')) {
        if !word.is_empty() {
            return files.iter().find(|f| f.starts_with(word) && f.len() > word.len()).map(|f| f[word.len()..].to_string());
        }
    }
    history_newest_first.iter().find(|h| h.starts_with(input) && h.len() > input.len() && !h.contains('\n')).map(|h| h[input.len()..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn history_completes_the_newest_match() {
        let h = v(&["fix the parser bug", "fix the lexer", "add tests"]);
        assert_eq!(ghost_suffix("fix the", &h, &[]), Some(" parser bug".into()));
        assert_eq!(ghost_suffix("add", &h, &[]), Some(" tests".into()));
        assert_eq!(ghost_suffix("add tests", &h, &[]), None, "exact match has nothing to add");
        assert_eq!(ghost_suffix("zzz", &h, &[]), None);
        assert_eq!(ghost_suffix("", &h, &[]), None);
        assert_eq!(ghost_suffix("  ", &h, &[]), None);
    }

    #[test]
    fn at_words_complete_files() {
        let f = v(&["src/main.rs", "src/lib.rs"]);
        assert_eq!(ghost_suffix("look at @src/m", &[], &f), Some("ain.rs".into()));
        assert_eq!(ghost_suffix("look at @src/main.rs", &[], &f), None);
        assert_eq!(ghost_suffix("look at @", &v(&["look at @x"]), &f), Some("x".into()), "a bare @ falls back to history");
    }

    #[test]
    fn multi_line_history_is_not_ghosted() {
        assert_eq!(ghost_suffix("a", &v(&["a\nb"]), &[]), None);
    }
}
