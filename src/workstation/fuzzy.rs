//! Fuzzy file finding for `@`: the letters typed must appear in the path in order, and the
//! score prefers runs, word starts and the file name over the folders.

/// How well `query` matches `path` (higher is better), or `None` when its letters do not all
/// appear, in order, in the path. Case does not matter.
pub fn score(query: &str, path: &str) -> Option<i64> {
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if q.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = path.chars().collect();
    let name_start = path.rfind('/').map_or(0, |i| path[..=i].chars().count());
    let (mut qi, mut total, mut prev) = (0, 0i64, None::<usize>);
    for (i, c) in chars.iter().enumerate() {
        if qi == q.len() {
            break;
        }
        if c.to_lowercase().next() != Some(q[qi]) {
            continue;
        }
        let mut gain = 1;
        if i > 0 && prev == Some(i - 1) {
            gain += 5;
        }
        if i == 0 || matches!(chars[i - 1], '/' | '.' | '_' | '-') {
            gain += 4;
        }
        if i >= name_start {
            gain += 2;
        }
        total += gain;
        prev = Some(i);
        qi += 1;
    }
    // Shorter paths win a tie: less of the match is spread over folders.
    (qi == q.len()).then_some(total * 100 - chars.len() as i64)
}

/// The best `limit` of `paths` for `query`, best first; equal scores keep their order.
pub fn rank<'a>(query: &str, paths: &'a [String], limit: usize) -> Vec<&'a String> {
    let mut hits: Vec<(i64, usize, &String)> = paths.iter().enumerate().filter_map(|(i, p)| score(query, p).map(|s| (s, i, p))).collect();
    hits.sort_by_key(|(s, i, _)| (std::cmp::Reverse(*s), *i));
    hits.into_iter().take(limit).map(|(_, _, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<String> {
        ["src/tui/controller.rs", "src/tui/app.rs", "src/uifmt/commands.rs", "docs/architecture/overview.md", "README.md"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn letters_must_appear_in_order() {
        assert!(score("ctl", "src/tui/controller.rs").is_some());
        assert!(score("ltc", "src/tui/controller.rs").is_none());
        assert_eq!(score("", "anything"), Some(0));
        assert!(score("CTL", "src/tui/controller.rs").is_some(), "case does not matter");
    }

    #[test]
    fn file_names_runs_and_word_starts_win() {
        let p = paths();
        assert_eq!(rank("app", &p, 3)[0], "src/tui/app.rs");
        assert_eq!(rank("cmds", &p, 3)[0], "src/uifmt/commands.rs");
        assert_eq!(rank("readme", &p, 3), vec!["README.md"]);
        assert!(rank("zzz", &p, 3).is_empty());
        assert_eq!(rank("rs", &p, 2).len(), 2);
    }
}
