//! The "always" rule for a Bash command, or why there is none.

use crate::domain::command_class::*;
use crate::domain::refusal::Refusal;
use crate::domain::shell_words::{split_chain, without_time_wrapper};

/// `Bash(cargo test:*)` for `cargo test --quiet`: the program as typed and, when the next word
/// looks like a subcommand (not a flag or a path), that too.
pub(super) fn bash_rule(command: &str) -> Result<String, Refusal> {
    if command.chars().any(is_shell_syntax) {
        return Err(Refusal::ShellSyntax);
    }
    // A word wholly in quotes is the word (`'1,80p'`); the chain splitter has done this already.
    let unquoted = |w: &'_ str| match (w.chars().next(), w.chars().last()) {
        (Some(open @ ('\'' | '"')), Some(close)) if w.len() >= 2 && open == close && !w[1..w.len() - 1].contains(open) => w[1..w.len() - 1].to_string(),
        _ => w.to_string(),
    };
    let all: Vec<String> = command.split([' ', '\t']).filter(|w| !w.is_empty()).map(unquoted).collect();
    let all: Vec<&str> = all.iter().map(String::as_str).collect();
    // The rule is the exact command: `sed -n:*` would also allow `sed -n 1e cmd`.
    if is_listing(&all) {
        return Ok(format!("Bash({})", all.join(" ")));
    }
    if all.first() == Some(&"sed") {
        return Err(Refusal::RunsOtherCommands);
    }
    let mut words = command.split([' ', '\t']).filter(|w| !w.is_empty());
    let program = words.next().filter(|w| is_plain_program(w)).ok_or(Refusal::NotPlainProgram)?;
    if program.contains('/') && !program.starts_with('/') {
        return Err(Refusal::ProjectProgram);
    }
    classify(program)?;
    let (dir, name) = program.rsplit_once('/').unwrap_or(("", program));
    if program.contains('/') && !SYSTEM_BIN_DIRS.contains(&dir) {
        return Err(Refusal::ProjectProgram);
    }
    let rest: Vec<&str> = words.collect();
    let go_exec_flag = |word: &str| {
        let flag = word.split('=').next().unwrap_or(word);
        name == "go" && ["-toolexec", "-vettool", "-exec", "--toolexec", "--vettool", "--exec"].contains(&flag)
    };
    let has_dangerous_flag = if command.contains(['\'', '"']) {
        split_chain(command)?.iter().flat_map(|segment| segment.iter().skip(1)).any(|word| is_dangerous_flag(&word.text) || go_exec_flag(&word.text))
    } else {
        all.iter().skip(1).any(|word| is_dangerous_flag(word) || go_exec_flag(word))
    };
    if has_dangerous_flag {
        return Err(Refusal::TooBroad);
    }
    let sub = rest.first().copied().filter(|w| is_subcommand(w));
    match SUBCOMMANDS.iter().find(|(p, _)| *p == name) {
        Some((_, allowed)) => {
            let sub = sub.ok_or(Refusal::NeedsSubcommand)?;
            if !allowed.contains(&sub) {
                return Err(if RUNNING_SUBCOMMANDS.contains(&sub) { Refusal::RunsOtherCommands } else { Refusal::NotListed });
            }
        }
        None if READ_ONLY.contains(&name) => {}
        None => return Err(Refusal::NotListed),
    }
    let writes_files = sub.is_some_and(|sub| WRITES_FILES.iter().any(|(p, subs)| *p == name && subs.contains(&sub)));
    if writes_files {
        // A `*` would make the exact rule a wildcard pattern again.
        if command.contains('*') {
            return Err(Refusal::NotExact);
        }
        return Ok(format!("Bash({})", command.trim_matches([' ', '\t'])));
    }
    Ok(match sub {
        Some(sub) => format!("Bash({program} {sub}:*)"),
        None => format!("Bash({program}:*)"),
    })
}

/// [`bash_rule`] for a whole command line: a simple command gets its one rule; a chain (see
/// [`split_chain`]) gets one rule per distinct simple command, and only when every one of them
/// has a safe prefix rule. A command whose rule would have to be the exact text (a subcommand
/// that writes files) cannot be part of a chain: its exact text is not a rule of its own.
pub(super) fn bash_rules(command: &str) -> Result<Vec<String>, Refusal> {
    if !command.chars().any(is_shell_syntax) {
        let words: Vec<String> = command.split([' ', '\t']).filter(|w| !w.is_empty()).map(String::from).collect();
        let inner = without_time_wrapper(&words);
        return if inner.len() == words.len() { bash_rule(command).map(|r| vec![r]) } else { bash_rule(&inner.join(" ")).map(|r| vec![r]) };
    }
    let mut rules: Vec<String> = Vec::new();
    for segment in split_chain(command)? {
        if segment.first().is_some_and(|w| w.quoted) {
            return Err(Refusal::NotPlainProgram);
        }
        // A word that would not survive as one plain word (it held spaces or shell characters
        // inside quotes) stands in as `ARG`: only the program, its subcommand and its flags
        // decide the rule, and a flag must be seen as it is.
        let words = segment
            .iter()
            .map(|w| {
                let plain = !w.text.is_empty() && !w.text.chars().any(|c| is_shell_syntax(c) || matches!(c, ' ' | '\t' | '"' | '\''));
                match (plain, w.text.starts_with('-')) {
                    (true, _) => Ok(w.text.clone()),
                    (false, true) => Err(Refusal::ShellSyntax),
                    (false, false) => Ok("ARG".to_string()),
                }
            })
            .collect::<Result<Vec<String>, Refusal>>()?;
        let command = without_time_wrapper(&words).join(" ");
        let rule = bash_rule(&command)?;
        // An exact-text rule (`git log --oneline -1`, a listing) is this command's own, as long
        // as every word of it is the real word and not a stand-in for one that held spaces.
        let literal = !words.iter().any(|w| w == "ARG");
        if !(rule.ends_with(":*)") || literal || is_listing(&command.split(' ').collect::<Vec<_>>())) {
            return Err(Refusal::ShellSyntax);
        }
        if !rules.contains(&rule) {
            rules.push(rule);
        }
    }
    Ok(rules)
}
