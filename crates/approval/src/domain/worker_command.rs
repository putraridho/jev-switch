//! Which shell commands a delegated worker may run when nobody can be asked.
//!
//! A worker runs in its own throwaway git worktree with no person watching. It may run a
//! command only when an "always" rule could be saved for it (see [`bash_rules`]: programs that
//! only read, the listed git inspection subcommands, the project's own build and test
//! commands), and when nothing in it reaches outside the worktree. What workers are told never
//! to do is left out even where a person could allow it: committing (`git add`, `git commit`)
//! and the network (`npm view`, `docker`, `kubectl`, package managers).

use crate::domain::bash_rule::bash_rules;
use crate::domain::shell_words::split_chain;

/// Programs, and their subcommands, a worker never runs without a person: the whole program
/// when the list is empty.
const NOT_FOR_WORKERS: &[(&str, &[&str])] = &[
    ("git", &["add", "commit"]),
    ("npm", &["view", "outdated"]),
    ("pnpm", &["outdated"]),
    ("yarn", &["outdated", "info"]),
    ("docker", &[]),
    ("podman", &[]),
    ("kubectl", &[]),
    ("pip", &[]),
    ("brew", &[]),
    ("apt", &[]),
    ("dnf", &[]),
];

/// Whether a worker may run `command` without asking anyone.
pub fn worker_may_run(command: &str) -> bool {
    let Ok(rules) = bash_rules(command) else { return false };
    let Ok(segments) = split_chain(command) else { return false };
    let stays_inside = segments.iter().all(|segment| segment.iter().skip(1).all(|word| !leaves_the_folder(&word.text)));
    stays_inside && !rules.iter().any(|rule| not_for_workers(rule))
}

/// `Bash(git commit:*)` or `Bash(git log --oneline -5)`: whether its program and subcommand
/// are in [`NOT_FOR_WORKERS`].
fn not_for_workers(rule: &str) -> bool {
    let inner = rule.strip_prefix("Bash(").and_then(|r| r.strip_suffix(')')).unwrap_or(rule);
    let inner = inner.strip_suffix(":*").unwrap_or(inner);
    let mut words = inner.split_whitespace();
    let program = words.next().unwrap_or_default();
    let program = program.rsplit('/').next().unwrap_or(program);
    let sub = words.next().unwrap_or_default();
    NOT_FOR_WORKERS.iter().any(|(p, subs)| *p == program && (subs.is_empty() || subs.contains(&sub)))
}

/// A word that names a place outside the current folder: an absolute path, the home folder,
/// or a step up (`..`), on its own, as a flag's value (`--file=/etc/x`), or glued to a short
/// flag (`-C/etc`).
fn leaves_the_folder(word: &str) -> bool {
    let outside = |s: &str| s.starts_with('/') || s.starts_with('~');
    let after_eq = word.split_once('=').map(|(_, v)| v);
    let glued = word.strip_prefix('-').filter(|w| !w.starts_with('-')).and_then(|w| w.get(1..));
    let steps_up = word.split(['/', '=']).any(|part| part == "..");
    outside(word) || after_eq.is_some_and(outside) || glued.is_some_and(outside) || steps_up
}
