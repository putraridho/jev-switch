//! The menu under the input box: slash commands as `/` is typed, a command's choices after
//! it, and files after `@`. Built fresh from the text before the cursor on every edit; pure,
//! so what each keystroke offers is a unit test.

use std::path::Path;

use crate::tui::input::line_editor::path_candidates;
use crate::tui::render as uifmt;
use crate::tui::render::{CompleteCtx, COMMANDS};
use crate::tui::input::fuzzy;

/// The most rows the menu shows at once; the selection scrolls through the rest.
pub const MENU_ROWS: usize = 8;

/// What accepting an item does after the text is put in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Then {
    /// Nothing more: keep typing (a file path, an argument that needs more).
    Stay,
    /// A command that takes arguments: a space follows, and its choices open.
    Arguments,
    /// A complete command or choice: Enter runs it at once.
    Run,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// What the row shows on the left (`/model`, `auto`, `src/lib.rs`).
    pub label: String,
    /// The arguments it takes (`[target]`, `[ask|edit|plan|auto]`); may be empty. Shown for the
    /// selected row only, so the label column stays as narrow as the names.
    pub usage: String,
    /// The dim description on the right; may be empty.
    pub detail: String,
    /// The text that replaces the word being completed.
    pub replacement: String,
    pub then: Then,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub items: Vec<Item>,
    pub selected: usize,
    /// Byte offset in the text where the word being completed starts.
    pub from: usize,
    /// The heading shown above the rows: `commands`, `/model`, `files`.
    pub title: String,
    /// The person moved the selection: Enter takes it even when it is not what was typed.
    pub navigated: bool,
}

impl Menu {
    pub fn up(&mut self) {
        self.navigated = true;
        let n = self.items.len();
        if n > 0 {
            self.selected = (self.selected + n - 1) % n;
        }
    }

    pub fn down(&mut self) {
        self.navigated = true;
        let n = self.items.len();
        if n > 0 {
            self.selected = (self.selected + 1) % n;
        }
    }

    pub fn current(&self) -> Option<&Item> {
        self.items.get(self.selected)
    }

    /// The first row shown, so the selection stays in view.
    pub fn first_shown(&self) -> usize {
        self.selected.saturating_sub(MENU_ROWS - 1)
    }
}

/// How well `name` matches what was typed: 0 exactly, 1 as a prefix, 2 as a substring, 3
/// for the letters in order; `None` for no match. Case is ignored.
fn rank(name: &str, typed: &str) -> Option<u8> {
    let (name, typed) = (name.to_lowercase(), typed.to_lowercase());
    if name == typed && !typed.is_empty() {
        return Some(0);
    }
    if name.starts_with(&typed) {
        return Some(1);
    }
    if name.contains(&typed) {
        return Some(2);
    }
    let mut letters = name.chars();
    typed.chars().all(|t| letters.any(|n| n == t)).then_some(3)
}

fn commands(typed: &str) -> Vec<Item> {
    let mut found: Vec<(u8, usize, Item)> = COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let r = rank(&c.name[1..], typed.trim_start_matches('/'))?;
            let then = if c.args.is_empty() { Then::Run } else { Then::Arguments };
            Some((r, i, Item { label: c.name.to_string(), usage: c.args.to_string(), detail: c.about.to_string(), replacement: c.name.to_string(), then }))
        })
        .collect();
    // Letters scattered through a name are a last resort: only when nothing matches better.
    if found.iter().any(|(r, _, _)| *r < 3) {
        found.retain(|(r, _, _)| *r < 3);
    }
    // Best match first; equal matches keep the table's order (grouped by purpose).
    found.sort_by_key(|(r, i, _)| (*r, *i));
    found.into_iter().map(|(_, _, item)| item).collect()
}

/// Whether a command's argument choice completes the line (`/mode plan`) or needs more after
/// it (`/permissions allow ...`).
fn choice_runs(command: &str, choice: &str) -> bool {
    !matches!((command, choice), ("/permissions", "allow" | "remove" | "dir") | ("/mcp", "login"))
}

/// `/skill `: its three words and every skill with its description; `/skill show `: the skills.
fn skill_menu(before: &str, ctx: &CompleteCtx, from: usize) -> Option<Menu> {
    let words: Vec<&str> = before.split_whitespace().collect();
    let typing_next = before.ends_with(char::is_whitespace);
    let index = if typing_next { words.len() } else { words.len() - 1 };
    let prefix = if typing_next { "" } else { words[words.len() - 1] };
    let skill_item = |s: &uifmt::SkillChoice, runs_now: bool| Item {
        label: s.name.clone(),
        usage: if runs_now { s.hint.clone() } else { String::new() },
        detail: s.description.clone(),
        replacement: s.name.clone(),
        then: if runs_now && !s.hint.is_empty() { Then::Arguments } else { Then::Run },
    };
    let items: Vec<Item> = match (index, words.get(1).copied()) {
        (1, _) => {
            let word = |w: &str, about: &str, then| Item { label: w.to_string(), usage: String::new(), detail: about.to_string(), replacement: w.to_string(), then };
            [word("list", "every skill, and how to use them", Then::Run), word("show", "read a skill before running it", Then::Arguments), word("new", "start a skill in this project", Then::Arguments)]
                .into_iter()
                .chain(ctx.skills.iter().map(|s| skill_item(s, true)))
                .filter(|i| i.replacement.starts_with(&prefix.to_lowercase()))
                .collect()
        }
        (2, Some("show")) => ctx.skills.iter().filter(|s| s.name.starts_with(&prefix.to_lowercase())).map(|s| skill_item(s, false)).collect(),
        _ => Vec::new(),
    };
    (!items.is_empty()).then(|| Menu { items, selected: 0, from, title: "skills".into(), navigated: false })
}

/// The menu for the text before the cursor, or `None` when there is nothing to offer.
/// `tracked` is the project's git-tracked files, for `@` paths.
pub fn build(before: &str, ctx: &CompleteCtx, project: &Path, tracked: &[String]) -> Option<Menu> {
    let from = uifmt::completion_start(before);
    let word = &before[from..];
    if let Some(path) = word.strip_prefix('@') {
        let mut items: Vec<Item> = path_candidates(path, project, tracked)
            .into_iter()
            .map(|p| Item { label: p.display, usage: String::new(), detail: String::new(), replacement: p.replacement, then: Then::Stay })
            .collect();
        // Nothing starts with what was typed: look for its letters scattered through the paths.
        if items.is_empty() && !path.is_empty() {
            items = fuzzy::rank(path, tracked, MENU_ROWS * 2)
                .into_iter()
                .map(|p| Item { label: p.clone(), usage: String::new(), detail: String::new(), replacement: format!("@{}", p.replace(' ', "\\ ")), then: Then::Stay })
                .collect();
        }
        return (!items.is_empty()).then(|| Menu { items, selected: 0, from, title: "files".into(), navigated: false });
    }
    if !before.starts_with('/') {
        return None;
    }
    if from == 0 {
        let items = commands(word);
        return (!items.is_empty()).then(|| Menu { items, selected: 0, from, title: "commands".into(), navigated: false });
    }
    let command = before.split_whitespace().next().unwrap_or_default();
    if command == "/skill" {
        return skill_menu(before, ctx, from);
    }
    let items: Vec<Item> = uifmt::complete(before, ctx)
        .into_iter()
        .map(|c| {
            let then = if choice_runs(command, &c.replacement) { Then::Run } else { Then::Arguments };
            Item { label: c.display, usage: String::new(), detail: String::new(), replacement: c.replacement, then }
        })
        .collect();
    (!items.is_empty()).then(|| Menu { items, selected: 0, from, title: command.to_string(), navigated: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(before: &str) -> Option<Menu> {
        let ctx = CompleteCtx { ready_providers: vec!["claude".into(), "codex".into()], models: vec!["claude:claude-opus-5-5".into()], changed_files: vec!["src/a.rs".into()], skills: vec![uifmt::SkillChoice { name: "db".into(), description: "Query the db".into(), hint: "[table]".into() }, uifmt::SkillChoice { name: "lint".into(), ..Default::default() }] };
        build(before, &ctx, Path::new("/nonexistent-project"), &["src/lib.rs".into(), "src/tui/menu.rs".into()])
    }

    fn labels(m: &Menu) -> Vec<&str> {
        m.items.iter().map(|i| i.replacement.as_str()).collect()
    }

    #[test]
    fn a_slash_lists_every_command_in_table_order_with_its_description() {
        let m = menu("/").expect("a menu");
        assert_eq!(m.items.len(), COMMANDS.len());
        assert_eq!(m.items[0].replacement, "/model", "grouped by purpose, not alphabetical: /clear is not first");
        assert!(m.items[0].detail.contains("Pin a model"));
        assert_eq!(m.title, "commands");
    }

    #[test]
    fn typing_narrows_prefix_first_then_substring_then_letters_in_order() {
        let m = menu("/re").expect("a menu");
        assert_eq!(&labels(&m)[..3], &["/rename", "/resume", "/rewind"]);
        let m = menu("/dif").expect("a menu");
        assert_eq!(labels(&m), vec!["/diff"]);
        let m = menu("/prv").expect("letters in order");
        assert!(labels(&m).contains(&"/provider") && labels(&m).contains(&"/providers"));
        let m = menu("/he").expect("a menu");
        assert_eq!(labels(&m)[0], "/help", "a prefix match comes before orc-he-strate");
        let m = menu("/hl").expect("letters in order");
        assert_eq!(labels(&m), vec!["/help"]);
        assert!(menu("/zzz").is_none());
    }

    #[test]
    fn commands_say_whether_they_run_or_take_arguments() {
        let m = menu("/hel").expect("a menu");
        assert_eq!(m.items[0].then, Then::Run);
        let m = menu("/mode").expect("a menu");
        assert_eq!(m.items[0].then, Then::Arguments);
        assert_eq!((m.items[0].label.as_str(), m.items[0].usage.as_str()), ("/mode", "[ask|edit|plan|auto]"));
    }

    #[test]
    fn after_a_command_its_choices_are_offered() {
        let m = menu("/mode ").expect("choices");
        assert_eq!(labels(&m), vec!["ask", "edit", "plan", "auto"]);
        assert_eq!(m.from, 6);
        assert!(m.items.iter().all(|i| i.then == Then::Run));
        let m = menu("/model co").expect("choices");
        assert_eq!(labels(&m), vec!["codex"]);
        let m = menu("/permissions ").expect("choices");
        assert_eq!(m.items[0].then, Then::Arguments, "`allow` needs a rule after it");
    }

    #[test]
    fn skill_offers_its_words_then_the_skills_with_their_descriptions() {
        let m = menu("/skill ").expect("choices");
        assert_eq!(labels(&m), vec!["list", "show", "new", "db", "lint"]);
        assert_eq!(m.title, "skills");
        assert_eq!((m.items[3].label.as_str(), m.items[3].usage.as_str(), m.items[3].detail.as_str(), m.items[3].then), ("db", "[table]", "Query the db", Then::Arguments));
        assert_eq!(menu("/skill show ").expect("skills").items[0].usage, "", "only a skill run now takes arguments");
        assert_eq!(m.items[4].then, Then::Run, "a skill with no arguments runs at once");
        assert_eq!(labels(&menu("/skill l").unwrap()), vec!["list", "lint"]);
        let m = menu("/skill show ").expect("skills only");
        assert_eq!(labels(&m), vec!["db", "lint"]);
        assert!(m.items.iter().all(|i| i.then == Then::Run));
        assert_eq!(menu("/skill show d").unwrap().from, 12);
        assert!(menu("/skill db ").is_none() && menu("/skill zzz").is_none());
    }

    #[test]
    fn an_at_word_offers_tracked_files_and_prose_offers_nothing() {
        let m = menu("look at @men").expect("files");
        assert_eq!(m.from, 8);
        assert_eq!(labels(&m), vec!["@src/tui/menu.rs"]);
        assert!(menu("just words").is_none());
    }

    #[test]
    fn scattered_letters_find_a_file_when_no_name_starts_with_them() {
        let m = menu("see @tm").expect("fuzzy files");
        assert_eq!(labels(&m), vec!["@src/tui/menu.rs"]);
        assert_eq!(m.items[0].replacement, "@src/tui/menu.rs");
        assert!(menu("see @qqq").is_none());
    }

    #[test]
    fn the_selection_wraps_and_stays_in_view() {
        let mut m = menu("/").expect("a menu");
        m.up();
        assert_eq!(m.selected, m.items.len() - 1);
        assert_eq!(m.first_shown(), m.items.len() - MENU_ROWS);
        m.down();
        assert_eq!((m.selected, m.first_shown()), (0, 0));
    }
}
