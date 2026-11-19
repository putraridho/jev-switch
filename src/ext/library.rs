//! Custom slash commands, skills and recipes written as markdown files.
//!
//! * a command is `<dir>/commands/<name>.md`: the body is the prompt, `$ARGUMENTS` (or `$1`..`$9`)
//!   is what was typed after `/<name>`;
//! * a skill is `<dir>/skills/<name>/SKILL.md`, used the same way: a described piece of know-how
//!   the person calls by name;
//! * a recipe is `<dir>/recipes/<name>.md`: every `## heading` starts a step, and the steps run
//!   one after the other, each as its own turn, stopping at the first that does not answer.
//!
//! `<dir>` is searched in this order, the first definition of a name wins: the project's `.jev`
//! (`.claude/commands` is read too, for commands people already wrote), the user's config
//! folder, then the installed extensions. A name may not shadow a built-in command.

use std::path::{Path, PathBuf};

use super::frontmatter::{self, Meta};
use super::Places;
use crate::mode::Mode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Command,
    Skill,
    Recipe,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Command, Kind::Skill, Kind::Recipe];

    pub fn dir(self) -> &'static str {
        match self {
            Kind::Command => "commands",
            Kind::Skill => "skills",
            Kind::Recipe => "recipes",
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Kind::Command => "command",
            Kind::Skill => "skill",
            Kind::Recipe => "recipe",
        }
    }
}

/// Where an entry came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Project,
    User,
    Extension(String),
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::Project => f.write_str("project"),
            Origin::User => f.write_str("user"),
            Origin::Extension(name) => write!(f, "extension {name}"),
        }
    }
}

/// One step of an entry: a command or skill has exactly one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub origin: Origin,
    pub meta: Meta,
    pub steps: Vec<Step>,
}

/// What running an entry asks of the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// How the turns are named in the session log, e.g. `/review-pr 12`.
    pub label: String,
    pub mode: Option<Mode>,
    /// The prompts to run in order, each with the title to show for it.
    pub prompts: Vec<(String, String)>,
}

/// Fill `$ARGUMENTS` and `$1`..`$9` in `body`. Arguments given to a body that never mentions
/// them are appended, so they are not lost.
pub fn expand(body: &str, args: &str) -> String {
    let args = args.trim();
    let words: Vec<&str> = args.split_whitespace().collect();
    let mentions = body.contains("$ARGUMENTS") || (1..=9).any(|n| body.contains(&format!("${n}")));
    // One pass over the body, so text that was substituted in is never scanned again: a
    // typed `$1` or `$5` stays as typed.
    let mut out = String::with_capacity(body.len() + args.len());
    let mut rest = body;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if let Some(after) = tail.strip_prefix("$ARGUMENTS") {
            out.push_str(args);
            rest = after;
        } else if let Some(n) = tail[1..].chars().next().and_then(|c| c.to_digit(10)).filter(|n| *n >= 1) {
            out.push_str(words.get(n as usize - 1).copied().unwrap_or(""));
            rest = &tail[2..];
        } else {
            out.push('$');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    if !mentions && !args.is_empty() {
        out.push_str(&format!("\n\nArguments: {args}"));
    }
    out
}

impl Entry {
    pub fn invoke(&self, args: &str) -> Invocation {
        let label = if args.trim().is_empty() { format!("/{}", self.name) } else { format!("/{} {}", self.name, args.trim()) };
        let prompts = self.steps.iter().map(|s| (s.title.clone(), expand(&s.body, args))).collect();
        Invocation { label, mode: self.meta.mode, prompts }
    }

    /// `/name [hint]`, the way a listing shows it.
    pub fn usage(&self) -> String {
        if self.meta.hint.is_empty() {
            format!("/{}", self.name)
        } else {
            format!("/{} {}", self.name, self.meta.hint)
        }
    }
}

/// Split a recipe body into steps at `## ` headings; text before the first heading is context
/// that goes at the head of the first step. No headings: one step.
fn recipe_steps(body: &str, name: &str) -> Vec<Step> {
    let mut preamble = String::new();
    let mut steps: Vec<Step> = Vec::new();
    for line in body.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            steps.push(Step { title: title.trim().to_string(), body: String::new() });
        } else if let Some(step) = steps.last_mut() {
            step.body.push_str(line);
            step.body.push('\n');
        } else {
            preamble.push_str(line);
            preamble.push('\n');
        }
    }
    if steps.is_empty() {
        return vec![Step { title: name.to_string(), body: body.trim().to_string() }];
    }
    for step in &mut steps {
        step.body = step.body.trim().to_string();
    }
    let preamble = preamble.trim();
    if !preamble.is_empty() {
        steps[0].body = format!("{preamble}\n\n{}", steps[0].body);
    }
    steps
}

/// A name that can be typed after a slash: lowercase letters, digits, `-` and `_`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[derive(Debug, Clone, Default)]
pub struct Library {
    pub entries: Vec<Entry>,
    /// Files that were skipped and why, one line each.
    pub problems: Vec<String>,
}

struct Root {
    dir: PathBuf,
    origin: Origin,
}

/// Where definitions are looked for, first one holding a name wins: the person's own, then
/// the extensions they installed, then the project's. A repository cannot take over a name
/// the person already uses (`/deploy` doing something else than they expect).
fn roots(places: &Places, kind: Kind) -> Vec<Root> {
    let mut roots = Vec::new();
    if let Some(user) = &places.user {
        roots.push(Root { dir: user.join(kind.dir()), origin: Origin::User });
        for ext in super::market::installed_dirs(user) {
            let name = ext.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            roots.push(Root { dir: ext.join(kind.dir()), origin: Origin::Extension(name) });
        }
    }
    roots.push(Root { dir: places.project.join(".jev").join(kind.dir()), origin: Origin::Project });
    if kind == Kind::Command {
        roots.push(Root { dir: places.project.join(".claude").join("commands"), origin: Origin::Project });
    }
    roots
}

/// The files of one root: `(name, path)`. Skills are folders holding `SKILL.md`.
fn candidates(root: &Root, kind: Kind) -> Vec<(String, PathBuf)> {
    let Ok(read) = std::fs::read_dir(&root.dir) else { return Vec::new() };
    let mut found: Vec<(String, PathBuf)> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            match kind {
                Kind::Skill if path.is_dir() => {
                    let name = path.file_name()?.to_string_lossy().into_owned();
                    Some((name, path.join("SKILL.md")))
                }
                Kind::Command | Kind::Recipe if path.extension().is_some_and(|x| x == "md") => {
                    Some((path.file_stem()?.to_string_lossy().into_owned(), path))
                }
                _ => None,
            }
        })
        .collect();
    found.sort();
    found
}

impl Library {
    pub fn discover(places: &Places) -> Self {
        let mut lib = Library::default();
        let builtin = crate::uifmt::command_names();
        for kind in Kind::ALL {
            for root in roots(places, kind) {
                for (name, path) in candidates(&root, kind) {
                    lib.load(kind, &root.origin, &name, &path, &builtin);
                }
            }
        }
        lib
    }

    fn load(&mut self, kind: Kind, origin: &Origin, name: &str, path: &Path, builtin: &[&str]) {
        let shown = path.display();
        if !valid_name(name) {
            self.problems.push(format!("{shown}: `{name}` is not a usable name (lowercase letters, digits, - and _)"));
            return;
        }
        if builtin.contains(&format!("/{name}").as_str()) {
            self.problems.push(format!("{shown}: /{name} is a built-in command and cannot be replaced"));
            return;
        }
        if let Some(held) = self.entries.iter().find(|e| e.name == name) {
            // An earlier root already holds the name; say so when the project's is the one hidden.
            if *origin == Origin::Project && held.origin != Origin::Project {
                self.problems.push(format!("{shown}: /{name} is already defined in your own files, which win"));
            }
            return;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.problems.push(format!("{shown}: {e}"));
                return;
            }
        };
        let doc = match frontmatter::parse(&text) {
            Ok(d) => d,
            Err(e) => {
                self.problems.push(format!("{shown}: {e}"));
                return;
            }
        };
        if doc.body.is_empty() {
            self.problems.push(format!("{shown}: nothing to run, the file has no body"));
            return;
        }
        let steps = match kind {
            Kind::Recipe => recipe_steps(&doc.body, name),
            Kind::Command | Kind::Skill => vec![Step { title: name.to_string(), body: doc.body }],
        };
        self.entries.push(Entry { name: name.to_string(), kind, origin: origin.clone(), meta: doc.meta, steps });
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |e| e.kind == kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-lib-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn arguments_fill_the_placeholders_or_are_appended() {
        assert_eq!(expand("fix $ARGUMENTS now", " the parser "), "fix the parser now");
        assert_eq!(expand("compare $1 with $2 ($3)", "a b"), "compare a with b ()");
        assert_eq!(expand("run the tests", "quickly"), "run the tests\n\nArguments: quickly");
        assert_eq!(expand("run the tests", ""), "run the tests");
    }

    #[test]
    fn typed_arguments_are_not_expanded_again() {
        assert_eq!(expand("note: $ARGUMENTS", "pay $1 now"), "note: pay $1 now");
        assert_eq!(expand("$1 costs $ARGUMENTS", "milk $5"), "milk costs milk $5");
        assert_eq!(expand("cost: $ and $x $", ""), "cost: $ and $x $");
    }

    #[test]
    fn recipes_split_into_steps_with_the_preamble_on_the_first() {
        let steps = recipe_steps("Context line.\n\n## Find\nlook\n\n## Fix\nfix $ARGUMENTS\n", "r");
        assert_eq!(steps.len(), 2);
        assert_eq!((steps[0].title.as_str(), steps[0].body.as_str()), ("Find", "Context line.\n\nlook"));
        assert_eq!((steps[1].title.as_str(), steps[1].body.as_str()), ("Fix", "fix $ARGUMENTS"));
        assert_eq!(recipe_steps("one prompt", "solo"), vec![Step { title: "solo".into(), body: "one prompt".into() }]);
    }

    #[test]
    fn discovery_prefers_the_persons_own_and_reports_what_it_skipped() {
        let root = temp("discover");
        let (project, user) = (root.join("proj"), root.join("cfg"));
        write(&project.join(".jev/commands/ship.md"), "---\ndescription: Ship it\nhint: [tag]\n---\nTag $1 and push.");
        write(&project.join(".claude/commands/legacy.md"), "Old prompt $ARGUMENTS");
        write(&user.join("commands/ship.md"), "user version");
        write(&user.join("commands/help.md"), "shadows a built-in");
        write(&user.join("commands/Bad Name.md"), "x");
        write(&user.join("commands/empty.md"), "---\ndescription: none\n---\n");
        write(&user.join("skills/db/SKILL.md"), "---\ndescription: Query the db\n---\nUse psql.");
        write(&user.join("recipes/fix-tests.md"), "---\nmode: edit\n---\n## Run\nrun tests\n## Fix\nfix them");
        let lib = Library::discover(&Places { project, user: Some(user) });
        // The repository cannot take over a name the person already uses.
        let ship = lib.find("ship").unwrap();
        assert_eq!((ship.origin.clone(), ship.usage().as_str()), (Origin::User, "/ship"));
        assert!(lib.problems.iter().any(|p| p.contains("/ship is already defined in your own files")), "{:?}", lib.problems);
        assert_eq!(lib.find("legacy").unwrap().kind, Kind::Command);
        assert_eq!(lib.find("db").unwrap().kind, Kind::Skill);
        let recipe = lib.find("fix-tests").unwrap();
        assert_eq!((recipe.kind, recipe.steps.len(), recipe.meta.mode), (Kind::Recipe, 2, Some(Mode::Edit)));
        assert!(lib.find("help").is_none());
        let problems = lib.problems.join("\n");
        assert!(problems.contains("built-in") && problems.contains("not a usable name") && problems.contains("no body"), "{problems}");
        assert_eq!(lib.of_kind(Kind::Command).count(), 2);
    }

    #[test]
    fn invoking_expands_every_step_and_labels_the_turns() {
        let root = temp("invoke");
        write(&root.join("proj/.jev/recipes/r.md"), "## One\ndo $1\n## Two\nthen $ARGUMENTS");
        let lib = Library::discover(&Places { project: root.join("proj"), user: None });
        let inv = lib.find("r").unwrap().invoke("x y");
        assert_eq!(inv.label, "/r x y");
        assert_eq!(inv.prompts, vec![("One".to_string(), "do x".to_string()), ("Two".to_string(), "then x y".to_string())]);
    }
}
