//! The Ctrl-K command palette: fuzzy search over commands, sessions, files and settings.

/// What an entry opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Command,
    Session,
    File,
    Setting,
}

impl Kind {
    pub fn tag(self) -> &'static str {
        match self {
            Self::Command => "cmd",
            Self::Session => "session",
            Self::File => "file",
            Self::Setting => "setting",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub label: String,
}

/// The score of `query` as a subsequence of `text` (case-insensitive), or `None` when the
/// letters do not appear in order. Higher is better: a match at the start of the text or of
/// a word, and letters that follow each other, score more; shorter texts win ties.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut ti, mut score, mut prev) = (0, 0i64, None::<usize>);
    for qc in &q {
        let at = (ti..t.len()).find(|&i| t[i] == *qc)?;
        score += 10;
        if prev.is_some_and(|p| p + 1 == at) {
            score += 15;
        }
        if at == 0 || (at == 1 && t[0] == '/') {
            score += 20;
        } else if matches!(t[at - 1], ' ' | '/' | '-' | '_' | '.' | ':') {
            score += 12;
        }
        prev = Some(at);
        ti = at + 1;
    }
    Some(score - t.len() as i64 / 4)
}

/// The entries that match `query`, best first (ties keep the input order); every entry for
/// an empty query. At most `limit`.
pub fn rank(query: &str, entries: &[Entry], limit: usize) -> Vec<Entry> {
    let mut scored: Vec<(i64, usize, &Entry)> = entries.iter().enumerate().filter_map(|(i, e)| fuzzy_score(query, &e.label).map(|s| (s, i, e))).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().take(limit).map(|(_, _, e)| e.clone()).collect()
}

/// A palette row: `cmd      /model`.
pub fn row(e: &Entry) -> String {
    format!("{:<8} {}", e.kind.tag(), e.label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(kind: Kind, label: &str) -> Entry {
        Entry { kind, label: label.into() }
    }

    #[test]
    fn subsequence_matching() {
        assert!(fuzzy_score("mdl", "/model").is_some());
        assert!(fuzzy_score("MODEL", "/model").is_some());
        assert_eq!(fuzzy_score("lmd", "/model"), None);
        assert_eq!(fuzzy_score("xyz", "/model"), None);
        assert!(fuzzy_score("", "anything").is_some());
    }

    #[test]
    fn word_starts_and_runs_rank_higher() {
        let entries = [e(Kind::File, "src/lib.rs"), e(Kind::Command, "/resume"), e(Kind::Session, "parser rewrite"), e(Kind::Setting, "reasoning effort")];
        let ranked = rank("re", &entries, 10);
        assert_eq!(ranked[0].label, "/resume");
        assert!(ranked.iter().any(|x| x.label == "reasoning effort"));
    }

    #[test]
    fn limit_and_empty_query() {
        let entries = [e(Kind::Command, "/a"), e(Kind::Command, "/b"), e(Kind::Command, "/c")];
        assert_eq!(rank("", &entries, 2).iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), ["/a", "/b"]);
        assert!(rank("zz", &entries, 5).is_empty());
    }

    #[test]
    fn rows_align_the_kind() {
        assert_eq!(row(&e(Kind::Command, "/model")), "cmd      /model");
        assert_eq!(row(&e(Kind::Session, "x")), "session  x");
    }
}
