//! Routing rules the user writes: "migrations go to codex", "never deepseek for security".
//!
//! One rule per line of `routing.rules` (user file in the state folder, then the project's
//! `.jev/routing.rules`; `#` starts a comment):
//!
//! ```text
//! prefer codex,claude when prompt contains "migration"
//! avoid deepseek when task is security
//! only claude when words > 400
//! prefer gemini when path ui/**
//! avoid grok
//! ```
//!
//! `prefer` narrows the choice to those providers when at least one of them is usable (else
//! it changes nothing), `avoid` drops them unless that would leave nothing, and `only` is
//! strict: when none of those providers can serve, the turn is refused and says why. Rules
//! apply in file order, each to what the earlier ones left. A `/model` pin skips them.

use crate::shared::ProviderId;
use crate::catalog::domain::model_table::{TaskType, Usable};
use crate::shared::domain::path_text::{glob_matches, mentioned_paths};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Prefer,
    Avoid,
    Only,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Condition {
    Always,
    PromptContains(String),
    Task(TaskType),
    WordsOver(usize),
    WordsUnder(usize),
    Path(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    action: Action,
    providers: Vec<ProviderId>,
    when: Condition,
    text: String,
}

/// What a rule can look at.
pub struct Facts<'a> {
    pub prompt: &'a str,
    /// Jev's task type, when it judged the turn.
    pub task: Option<TaskType>,
    /// Paths the prompt names, see [`crate::shared::domain::path_text::mentioned_paths`].
    pub paths: Vec<String>,
}

impl<'a> Facts<'a> {
    pub fn new(root: &std::path::Path, prompt: &'a str, task: Option<TaskType>) -> Self {
        Self { prompt, task, paths: mentioned_paths(root, prompt) }
    }
}

impl Condition {
    fn holds(&self, facts: &Facts) -> bool {
        match self {
            Self::Always => true,
            Self::PromptContains(needle) => facts.prompt.to_lowercase().contains(needle),
            Self::Task(t) => facts.task == Some(*t),
            Self::WordsOver(n) => facts.prompt.split_whitespace().count() > *n,
            Self::WordsUnder(n) => facts.prompt.split_whitespace().count() < *n,
            Self::Path(glob) => facts.paths.iter().any(|p| glob_matches(glob, p)),
        }
    }
}

/// A problem on one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    rules: Vec<Rule>,
}

/// The providers left for the turn, and what the rules did.
#[derive(Debug, Clone)]
pub struct Applied {
    pub usable: Usable,
    /// One line per rule that fired, for `/why`-style output.
    pub notes: Vec<String>,
}

impl Rules {
    pub fn parse(text: &str) -> (Self, Vec<RuleError>) {
        let mut rules = Vec::new();
        let mut errors = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            match parse_rule(line) {
                Ok(rule) => rules.push(rule),
                Err(message) => errors.push(RuleError { line: i + 1, message }),
            }
        }
        (Self { rules }, errors)
    }

    /// Where the rule files are, in the order they apply: the user's, then the project's.
    pub fn file_paths(state_dir: Option<&std::path::Path>, project: &std::path::Path) -> Vec<std::path::PathBuf> {
        [state_dir.map(|d| d.join("routing.rules")), Some(project.join(".jev/routing.rules"))].into_iter().flatten().collect()
    }

    /// The rules of several files' texts, in order; later rules apply after earlier ones.
    /// Line numbers in errors are per file.
    pub fn parse_files<'t>(texts: impl IntoIterator<Item = &'t str>) -> (Self, Vec<RuleError>) {
        let mut all = Self::default();
        let mut errors = Vec::new();
        for text in texts {
            let (rules, errs) = Self::parse(text);
            all.rules.extend(rules.rules);
            errors.extend(errs);
        }
        (all, errors)
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().map(|r| r.text.as_str())
    }

    /// Narrow `usable` by every rule whose condition holds. `Err` is an `only` rule that
    /// no usable provider satisfies.
    pub fn apply(&self, usable: &Usable, facts: &Facts) -> Result<Applied, String> {
        let mut current = usable.clone();
        let mut notes = Vec::new();
        for rule in self.rules.iter().filter(|r| r.when.holds(facts)) {
            let kept = current.filter(|p, _| rule.providers.contains(&p));
            let without = current.filter(|p, _| !rule.providers.contains(&p));
            match rule.action {
                Action::Prefer if !kept.is_empty() => current = kept,
                Action::Avoid if !without.is_empty() => current = without,
                Action::Only if kept.is_empty() => return Err(format!("your routing rule `{}` allows only those providers, and none can take this turn now", rule.text)),
                Action::Only => current = kept,
                Action::Prefer | Action::Avoid => continue,
            }
            notes.push(format!("rule: {}", rule.text));
        }
        Ok(Applied { usable: current, notes })
    }
}

fn strip_comment(line: &str) -> &str {
    // A `#` inside a quoted phrase is part of the phrase.
    let mut in_quote = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            '#' if !in_quote => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse_rule(line: &str) -> Result<Rule, String> {
    let (head, cond) = match line.split_once(" when ") {
        Some((h, c)) => (h.trim(), Some(c.trim())),
        None => (line, None),
    };
    let mut words = head.split_whitespace();
    let action = match words.next() {
        Some("prefer") => Action::Prefer,
        Some("avoid") => Action::Avoid,
        Some("only") => Action::Only,
        _ => return Err("a rule starts with prefer, avoid or only".into()),
    };
    let list = words.next().ok_or("name the providers, like `prefer codex,claude`")?;
    if words.next().is_some() {
        return Err("expected `<action> <providers> [when <condition>]`".into());
    }
    let providers: Result<Vec<ProviderId>, String> = list.split(',').map(str::parse).collect();
    let when = cond.map_or(Ok(Condition::Always), parse_condition)?;
    Ok(Rule { action, providers: providers?, when, text: line.to_string() })
}

fn parse_condition(cond: &str) -> Result<Condition, String> {
    let mut parts = cond.splitn(3, char::is_whitespace);
    let (a, b) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let rest = parts.next().unwrap_or("").trim();
    match (a, b) {
        ("prompt", "contains") => {
            let phrase = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')).filter(|p| !p.is_empty());
            phrase.map(|p| Condition::PromptContains(p.to_lowercase())).ok_or_else(|| "put the phrase in double quotes: prompt contains \"migration\"".into())
        }
        ("task", "is") => rest.parse::<TaskType>().map(Condition::Task),
        ("words", ">") => rest.parse().map(Condition::WordsOver).map_err(|_| "words > needs a number".into()),
        ("words", "<") => rest.parse().map(Condition::WordsUnder).map_err(|_| "words < needs a number".into()),
        ("path", glob) if rest.is_empty() && !glob.is_empty() => Ok(Condition::Path(glob.to_string())),
        _ => Err("conditions: prompt contains \"text\", task is <type>, words > N, words < N, path <glob>".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn usable() -> Usable {
        Usable::all_of([ProviderId::Claude, ProviderId::Codex, ProviderId::DeepSeek])
    }

    fn providers(u: &Usable) -> Vec<ProviderId> {
        let mut v: Vec<ProviderId> = crate::shared::ALL_PROVIDERS.into_iter().filter(|p| !u.filter(|q, _| q == *p).is_empty()).collect();
        v.sort();
        v
    }

    fn rules(text: &str) -> Rules {
        let (r, errors) = Rules::parse(text);
        assert!(errors.is_empty(), "{errors:?}");
        r
    }

    fn facts<'a>(prompt: &'a str, task: Option<TaskType>) -> Facts<'a> {
        Facts::new(Path::new("/w"), prompt, task)
    }

    #[test]
    fn prefer_narrows_when_it_can_and_is_ignored_when_it_cannot() {
        let r = rules("prefer codex when prompt contains \"Migration\"");
        let a = r.apply(&usable(), &facts("write a migration for users", None)).unwrap();
        assert_eq!(providers(&a.usable), vec![ProviderId::Codex]);
        assert_eq!(a.notes, vec!["rule: prefer codex when prompt contains \"Migration\""]);
        // No match: untouched.
        assert_eq!(providers(&r.apply(&usable(), &facts("fix a typo", None)).unwrap().usable).len(), 3);
        // Preferred provider not usable: untouched, no note.
        let only_claude = Usable::all_of([ProviderId::Claude]);
        let a = r.apply(&only_claude, &facts("a migration", None)).unwrap();
        assert_eq!(providers(&a.usable), vec![ProviderId::Claude]);
        assert!(a.notes.is_empty());
    }

    #[test]
    fn avoid_drops_unless_it_would_leave_nothing() {
        let r = rules("avoid deepseek,codex when task is security");
        let a = r.apply(&usable(), &facts("x", Some(TaskType::Security))).unwrap();
        assert_eq!(providers(&a.usable), vec![ProviderId::Claude]);
        assert_eq!(providers(&r.apply(&usable(), &facts("x", Some(TaskType::Docs))).unwrap().usable).len(), 3);
        // Without Jev's judgment the task condition does not hold.
        assert_eq!(providers(&r.apply(&usable(), &facts("x", None)).unwrap().usable).len(), 3);
        let left = Usable::all_of([ProviderId::DeepSeek]);
        assert_eq!(providers(&r.apply(&left, &facts("x", Some(TaskType::Security))).unwrap().usable), vec![ProviderId::DeepSeek]);
    }

    #[test]
    fn only_is_strict() {
        let r = rules("only claude when words > 3");
        assert_eq!(providers(&r.apply(&usable(), &facts("one two three four", None)).unwrap().usable), vec![ProviderId::Claude]);
        assert_eq!(providers(&r.apply(&usable(), &facts("one two", None)).unwrap().usable).len(), 3);
        let err = r.apply(&Usable::all_of([ProviderId::Codex]), &facts("one two three four", None)).unwrap_err();
        assert!(err.contains("only claude when words > 3"), "{err}");
    }

    #[test]
    fn path_conditions_and_order() {
        let r = rules("prefer codex,claude when path ui/**\navoid claude when words < 100\nprefer deepseek");
        // ui path: narrow to codex+claude, then avoid claude, then prefer deepseek has none left: ignored.
        let a = r.apply(&usable(), &facts("fix @ui/button.tsx please", None)).unwrap();
        assert_eq!(providers(&a.usable), vec![ProviderId::Codex]);
        assert_eq!(a.notes.len(), 2);
        // Without the path: avoid claude, then prefer deepseek.
        let a = r.apply(&usable(), &facts("fix it", None)).unwrap();
        assert_eq!(providers(&a.usable), vec![ProviderId::DeepSeek]);
    }

    #[test]
    fn unconditional_rule_and_comments() {
        let r = rules("avoid grok  # slow here\n\n# nothing\nprefer claude when prompt contains \"a # b\"");
        assert_eq!(r.lines().count(), 2);
        let a = r.apply(&usable(), &facts("say a # b", None)).unwrap();
        assert_eq!(providers(&a.usable), vec![ProviderId::Claude]);
    }

    #[test]
    fn bad_lines_are_numbered_and_do_not_stop_the_rest() {
        let (r, errors) = Rules::parse("prefer nobody\nfly claude\nprefer claude when sunny\nonly claude when words > x\nprefer claude when prompt contains oops\nprefer codex");
        assert_eq!(errors.iter().map(|e| e.line).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5]);
        assert_eq!(r.lines().collect::<Vec<_>>(), vec!["prefer codex"]);
        assert!(Rules::default().is_empty());
    }
}
