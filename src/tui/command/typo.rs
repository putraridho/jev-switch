//! The spelling helpers behind the parser: the routing switch and "did you mean" for a mistyped command.

use crate::tui::render as uifmt;

/// Whether provider routing starts on: `JEV_PROVIDERS=off` (also `0`, `false`, `no`)
/// starts it off, anything else, including unset, leaves the default (on).
pub fn routing_enabled(env: Option<&str>) -> bool {
    !env.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no"))
}

/// Levenshtein distance between two short words, in characters.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diag + usize::from(ca != *cb)).min(above + 1).min(row[j] + 1);
            diag = above;
        }
    }
    row[b.len()]
}

/// The command a mistyped one most likely meant: one that starts with what was typed
/// (`/mod` is `/model`), else the nearest by edit distance when it is close enough to be
/// a typo and not a different word (`/provders` is `/providers`, `/xyz` is nothing).
pub fn closest_command(typed: &str) -> Option<&'static str> {
    let typed = typed.to_ascii_lowercase();
    let names = uifmt::command_names();
    if typed.len() > 2 {
        if let Some(prefixed) = names.iter().find(|n| n.starts_with(&typed)) {
            return Some(prefixed);
        }
    }
    names
        .into_iter()
        .map(|n| (edit_distance(&typed, n), n))
        .filter(|(d, _)| *d <= 2 && *d < typed.chars().count())
        .min_by_key(|(d, _)| *d)
        .map(|(_, n)| n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_is_on_unless_the_env_says_off() {
        assert!(routing_enabled(None));
        assert!(routing_enabled(Some("auto")));
        assert!(routing_enabled(Some("")));
        for off in ["off", "OFF", " 0 ", "false", "no"] {
            assert!(!routing_enabled(Some(off)), "{off}");
        }
    }

    #[test]
    fn edit_distance_counts_single_character_edits() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("/provders", "/providers"), 1);
        assert_eq!(edit_distance("/hlep", "/help"), 2);
    }

    #[test]
    fn the_closest_command_is_a_prefix_or_a_near_typo_and_never_a_stretch() {
        for (typed, want) in [
            ("/mod", Some("/model")),
            ("/prov", Some("/provider")),
            ("/provders", Some("/providers")),
            ("/hlep", Some("/help")),
            ("/dif", Some("/diff")),
            ("/staus", Some("/status")),
            ("/Model", Some("/model")),
            ("/xyzzy", None),
            ("/", None),
            ("/q", None),
        ] {
            assert_eq!(closest_command(typed), want, "{typed}");
        }
    }
}
