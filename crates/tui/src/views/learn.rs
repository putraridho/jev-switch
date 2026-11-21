//! Learning and help: `/help <words>` searches the commands and keys, `/tips` lists tips,
//! `/keys` (or `?` on an empty box) is the key cheat sheet, `/whatsnew` shows the changelog.
//! Every screen is a pure function of the command table and the embedded changelog.

use crate::render::{command_usage, display_width, truncate_to_width, wrap_to_width, Style, COMMANDS, KEYS, MIN_WIDTH};

/// The changelog, newest version first; each version starts with a `## <version>` line.
const CHANGELOG: &str = include_str!("../../../../docs/CHANGELOG.md");

/// What a learning command asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Learn {
    /// `/help <words>`.
    Search(String),
    /// `/tips`.
    Tips,
    /// `/keys`, or `?` on an empty box.
    Keys,
    /// `/whatsnew [all]`.
    WhatsNew { all: bool },
}

impl Learn {
    /// The learning command in a parsed line: `head` is the lower-cased command word and
    /// `rest` the text after it. `None` for any other command.
    pub fn parse(head: &str, rest: Option<&str>) -> Option<Result<Self, String>> {
        let bad = |name: &str| Err(format!("usage: {}", command_usage(name).unwrap_or_else(|| name.to_string())));
        Some(match (head, rest) {
            ("/help", Some(q)) => Ok(Self::Search(q.to_string())),
            ("/tips", None) => Ok(Self::Tips),
            ("/keys", None) => Ok(Self::Keys),
            ("/whatsnew", None) => Ok(Self::WhatsNew { all: false }),
            ("/whatsnew", Some("all")) => Ok(Self::WhatsNew { all: true }),
            ("/tips" | "/keys" | "/whatsnew", Some(_)) => bad(head),
            _ => return None,
        })
    }

    pub fn render(&self, style: Style, width: usize) -> String {
        match self {
            Self::Search(q) => help_search(q, style, width),
            Self::Tips => tips_text(style, width),
            Self::Keys => cheat_sheet(style, width),
            Self::WhatsNew { all } => whats_new(env!("CARGO_PKG_VERSION"), *all, style, width),
        }
    }
}

/// How well `words` (all of them, any case) match a text: 0 for no match.
fn relevance(words: &[String], name: &str, about: &str) -> u32 {
    let (name, about) = (name.to_lowercase(), about.to_lowercase());
    let mut score = 0;
    for w in words {
        score += if name.trim_start_matches('/') == w.trim_start_matches('/') {
            10
        } else if name.contains(w.as_str()) {
            6
        } else if about.contains(w.as_str()) {
            2
        } else {
            return 0;
        };
    }
    score
}

fn rows(entries: &[(String, &str)], style: Style, width: usize) -> Vec<String> {
    let width = width.max(MIN_WIDTH);
    let col = entries.iter().map(|(k, _)| display_width(k)).max().unwrap_or(0) + 2;
    let side = width >= 2 + col + 30;
    let mut lines = Vec::new();
    for (key, about) in entries {
        if side {
            for (i, part) in wrap_to_width(about, width - 2 - col).iter().enumerate() {
                let lead = if i == 0 { format!("  {}{}", style.cyan(key), " ".repeat(col - display_width(key))) } else { " ".repeat(2 + col) };
                lines.push(format!("{lead}{part}"));
            }
        } else {
            lines.push(truncate_to_width(&format!("  {}", style.cyan(key)), width));
            lines.extend(wrap_to_width(about, width - 4).iter().map(|p| format!("    {p}")));
        }
    }
    lines
}

/// `/help <words>`: the commands and keys that match every word, best first, with an
/// example when the command has one.
pub fn help_search(query: &str, style: Style, width: usize) -> String {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut hits: Vec<(u32, usize, String, &str)> = Vec::new();
    for (i, c) in COMMANDS.iter().enumerate() {
        let about = format!("{} {}", c.about, c.example);
        let score = relevance(&words, c.name, &about);
        if score > 0 {
            hits.push((score, i, command_usage(c.name).unwrap_or_else(|| c.name.to_string()), c.about));
        }
    }
    for (i, (key, what)) in KEYS.iter().enumerate() {
        let score = relevance(&words, key, what);
        if score > 0 {
            hits.push((score, COMMANDS.len() + i, (*key).to_string(), what));
        }
    }
    hits.sort_by_key(|(score, i, _, _)| (std::cmp::Reverse(*score), *i));
    if hits.is_empty() {
        return format!("  Nothing in /help matches \"{}\". Try one word, or /help for everything.", query.trim());
    }
    let entries: Vec<(String, &str)> = hits.into_iter().map(|(_, _, k, a)| (k, a)).collect();
    let mut lines = vec![style.bold(&format!("Help matching \"{}\"", query.trim()))];
    lines.extend(rows(&entries, style, width));
    lines.join("\n")
}

/// Short tips, each one a thing worth knowing that the screen does not already say.
pub const TIPS: [&str; 9] = [
    "Type / to see every command; keep typing to narrow the list.",
    "Type @ then a few letters of a file name: the letters may be scattered (`@ctl` finds `controller.rs`).",
    "Shift-Tab cycles the mode: ask, edit, plan, auto. Plan mode reads and thinks but changes nothing.",
    "/why explains why the router picked the last turn's model; /model pins one.",
    "/undo puts back the files the last turn changed; /diff shows them first.",
    "Esc stops a running turn and sends what you typed right after it.",
    "Start a command with ! to run it in the shell; the output goes with your next message.",
    "/compact frees context when the footer says it is filling up.",
    "Set JEV_VIM=1 for vim keys in the input box, JEV_REDUCED_MOTION=1 for a still spinner.",
];

/// The tip for a given day, so a hint can change daily without any state.
pub fn tip_for_day(day: u64) -> &'static str {
    TIPS[(day % TIPS.len() as u64) as usize]
}

pub fn tips_text(style: Style, width: usize) -> String {
    let width = width.max(MIN_WIDTH);
    let mut lines = vec![style.bold("Tips")];
    for (i, tip) in TIPS.iter().enumerate() {
        let lead = format!("{:>2}. ", i + 1);
        for (j, part) in wrap_to_width(tip, width - 6).iter().enumerate() {
            lines.push(format!("  {}{part}", if j == 0 { style.dim(&lead) } else { "    ".to_string() }));
        }
    }
    lines.extend(wrap_to_width("/keys shows the keys; /help <words> searches the commands.", width - 2).iter().map(|l| style.dim(&format!("  {l}"))));
    lines.join("\n")
}

/// The key cheat sheet: only the keys, with no commands to scroll past.
pub fn cheat_sheet(style: Style, width: usize) -> String {
    let entries: Vec<(String, &str)> = KEYS.iter().map(|(k, w)| ((*k).to_string(), *w)).collect();
    let mut lines = vec![style.bold("Keys")];
    lines.extend(rows(&entries, style, width));
    lines.join("\n")
}

/// The versions of a changelog as `(version, bullet lines)`, in file order.
fn versions(log: &str) -> Vec<(&str, Vec<&str>)> {
    let mut out: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in log.lines() {
        if let Some(v) = line.strip_prefix("## ") {
            out.push((v.trim(), Vec::new()));
        } else if let (Some(item), Some((_, items))) = (line.strip_prefix("- "), out.last_mut()) {
            items.push(item.trim());
        }
    }
    out
}

/// `/whatsnew`: what changed in `current` and, with `all`, in every version. The
/// unreleased entries count as part of the current version.
pub fn whats_new(current: &str, all: bool, style: Style, width: usize) -> String {
    whats_new_from(CHANGELOG, current, all, style, width)
}

fn whats_new_from(log: &str, current: &str, all: bool, style: Style, width: usize) -> String {
    let width = width.max(MIN_WIDTH);
    let mut lines = Vec::new();
    let found = versions(log);
    let unreleased_first = found.first().is_some_and(|(v, _)| *v == "Unreleased");
    for (i, (version, items)) in found.iter().enumerate() {
        // The newest entry, and the current version when entries not yet released sit above it.
        if !(all || i == 0 || (unreleased_first && *version == current)) {
            continue;
        }
        lines.push(style.bold(&if *version == "Unreleased" { format!("New since {current}") } else { format!("Version {version}") }));
        for item in items {
            for (j, part) in wrap_to_width(item, width - 4).iter().enumerate() {
                lines.push(format!("  {}{part}", if j == 0 { "• " } else { "  " }));
            }
        }
        lines.push(String::new());
    }
    lines.pop();
    if lines.is_empty() {
        return "  No changelog entries.".to_string();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFF: Style = Style { enabled: false, terminal: false };

    #[test]
    fn search_finds_by_name_then_by_description_and_keys() {
        let s = help_search("diff", OFF, 100);
        let first = s.lines().nth(1).unwrap_or_default();
        assert!(first.contains("/diff "), "{s}");
        assert!(s.contains("/undo") || s.contains("changed"), "{s}");
        let keys = help_search("vim", OFF, 100);
        assert!(keys.contains("Esc, then h j k l"), "{keys}");
        let all_words = help_search("copy answer", OFF, 100);
        assert!(all_words.contains("/copy") && !all_words.contains("/diff"), "{all_words}");
        assert!(help_search("zzzzqq", OFF, 100).contains("Nothing in /help matches"));
    }

    #[test]
    fn every_screen_fits_any_width_and_has_no_escapes_without_colour() {
        for w in [0, 20, 40, 80, 200] {
            for text in [help_search("a", OFF, w), tips_text(OFF, w), cheat_sheet(OFF, w), whats_new("0.2.0", true, OFF, w)] {
                assert!(!text.contains('\x1b'));
                for l in text.lines() {
                    assert!(display_width(l) <= w.max(MIN_WIDTH), "{l:?} at {w}");
                }
            }
        }
    }

    #[test]
    fn parse_takes_the_learning_commands_only() {
        assert_eq!(Learn::parse("/help", Some("diff")), Some(Ok(Learn::Search("diff".into()))));
        assert_eq!(Learn::parse("/tips", None), Some(Ok(Learn::Tips)));
        assert_eq!(Learn::parse("/whatsnew", Some("all")), Some(Ok(Learn::WhatsNew { all: true })));
        assert_eq!(Learn::parse("/keys", Some("x")), Some(Err("usage: /keys".into())));
        assert_eq!(Learn::parse("/whatsnew", Some("x")), Some(Err("usage: /whatsnew [all]".into())));
        assert_eq!(Learn::parse("/model", None), None);
    }

    #[test]
    fn whats_new_shows_the_unreleased_entries_then_everything_with_all() {
        let log = "## Unreleased\n\n- a thing\n\n## 0.2.0\n\n- old\n";
        let now = whats_new_from(log, "0.2.0", false, OFF, 80);
        assert!(now.contains("New since 0.2.0") && now.contains("a thing") && now.contains("Version 0.2.0"), "{now}");
        let other = whats_new_from(log, "0.3.0", false, OFF, 80);
        assert!(other.contains("a thing") && !other.contains("old"), "{other}");
        let all = whats_new_from(log, "0.2.0", true, OFF, 80);
        assert!(all.contains("Version 0.2.0") && all.contains("old"), "{all}");
        let plain = whats_new_from("## 0.2.0\n- old\n## 0.1.0\n- older\n", "0.2.0", false, OFF, 80);
        assert!(plain.contains("old") && !plain.contains("older"), "{plain}");
    }

    #[test]
    fn the_shipped_changelog_and_tips_are_well_formed() {
        assert!(versions(CHANGELOG).iter().all(|(_, items)| !items.is_empty()));
        assert!(TIPS.iter().all(|t| !t.is_empty()));
        assert_eq!(tip_for_day(0), tip_for_day(TIPS.len() as u64));
    }
}
