//! Layered settings and profiles.
//!
//! Settings are the `KEY=VALUE` names jev-code already reads from the environment
//! (`JEV_PROVIDERS`, `CODE_PERMISSION_MODE`, ...). Three files add to them, lowest first:
//!
//! 1. `<config folder>/config.env`: the person's defaults;
//! 2. `<config folder>/profiles/<name>.env`: a named bundle, chosen with `--profile <name>`
//!    or `JEV_PROFILE=<name>`;
//! 3. `<project>/.jev/config.env`: the project's own, limited to harmless names
//!    ([`PROJECT_KEYS`]): a repository must not pick the binaries that run, the permission
//!    mode or the model order for whoever opens it.
//!
//! The process environment always wins, and these files win over `~/.jev-router.env`. Keys
//! and tokens never come from these files (they would be committed or shared): they stay in
//! `~/.jev-router.env`. Everything here is pure over text, so each rule is a unit test;
//! [`Layered::load`] only reads the three files.

use std::collections::BTreeMap;
use std::path::Path;

use jev_approval::application::extensions::Places;

/// The only names a project's `.jev/config.env` may set.
pub const PROJECT_KEYS: [&str; 5] = ["JEV_PROVIDERS", "JEV_ORCHESTRATE", "JEV_BELL", "DOWNGRADE_AFTER", "CODE_STRONG_THRESHOLD"];

/// Switches a project may only turn off: turning on routing across providers or
/// orchestration sends the person's prompts further than their own `config.env` chose.
const PROJECT_OFF_ONLY: [&str; 2] = ["JEV_PROVIDERS", "JEV_ORCHESTRATE"];

fn is_off(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no")
}

/// Where a setting's value came from, lowest layer first.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    User,
    Profile(String),
    Project,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => f.write_str("config.env"),
            Self::Profile(name) => write!(f, "profile {name}"),
            Self::Project => f.write_str(".jev/config.env"),
        }
    }
}

/// One effective setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub key: String,
    pub value: String,
    pub source: Source,
}

/// The settings the layers give, and what was left out of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layered {
    /// In name order, each from the highest layer that sets it.
    pub settings: Vec<Setting>,
    /// Lines and names that were refused, ready to show.
    pub problems: Vec<String>,
}

/// Names that hold a secret: never read from a layer.
fn is_secret(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD"].iter().any(|w| upper.ends_with(w) || upper.contains(&format!("{w}_")))
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') && !key.starts_with(|c: char| c.is_ascii_digit())
}

/// `KEY=VALUE` lines: `#` comments, a leading `export `, one layer of quotes.
fn parse_pairs(text: &str) -> Vec<Result<(String, String), String>> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| {
            let line = line.strip_prefix("export ").unwrap_or(line).trim();
            let (key, value) = line.split_once('=').ok_or_else(|| format!("not KEY=VALUE: {line}"))?;
            let key = key.trim();
            if !valid_key(key) {
                return Err(format!("`{key}` is not a setting name (UPPER_CASE letters, digits, _)"));
            }
            Ok((key.to_string(), value.trim().trim_matches(|c| c == '"' || c == '\'').to_string()))
        })
        .collect()
}

impl Layered {
    /// Merge `layers`, lowest first; a later layer's value replaces an earlier one's.
    pub fn merge(layers: &[(Source, &str)]) -> Self {
        let mut winners: BTreeMap<String, Setting> = BTreeMap::new();
        let mut problems = Vec::new();
        for (source, text) in layers {
            for parsed in parse_pairs(text) {
                let (key, value) = match parsed {
                    Ok(pair) => pair,
                    Err(e) => {
                        problems.push(format!("{source}: {e}"));
                        continue;
                    }
                };
                if is_secret(&key) {
                    problems.push(format!("{source}: {key} is a secret and stays in ~/.jev-router.env"));
                } else if *source == Source::Project && !PROJECT_KEYS.contains(&key.as_str()) {
                    problems.push(format!("{source}: {key} may not be set by a project (allowed: {})", PROJECT_KEYS.join(", ")));
                } else if *source == Source::Project && PROJECT_OFF_ONLY.contains(&key.as_str()) && !is_off(&value) {
                    problems.push(format!("{source}: a project may only turn {key} off"));
                } else if !value.is_empty() {
                    winners.insert(key.clone(), Setting { key, value, source: source.clone() });
                }
            }
        }
        Self { settings: winners.into_values().collect(), problems }
    }

    /// Read the three files. `profile` is the chosen profile name, if any; a profile that
    /// is not valid or not there is a problem, not an error.
    pub fn load(places: &Places, profile: Option<&str>) -> Self {
        let read = |path: &Path| std::fs::read_to_string(path).ok();
        let user = places.user.as_deref();
        let user_text = user.and_then(|u| read(&u.join("config.env")));
        let mut notes = Vec::new();
        let profile_text = match (profile, user) {
            (Some(name), _) if !jev_approval::application::extension_library::valid_name(name) => {
                notes.push(format!("profile `{name}` is not a usable name (lowercase letters, digits, - and _)"));
                None
            }
            (Some(name), Some(u)) => {
                let text = read(&u.join("profiles").join(format!("{name}.env")));
                if text.is_none() {
                    notes.push(format!("no profile named {name} (looked for profiles/{name}.env in the config folder)"));
                }
                text.map(|t| (Source::Profile(name.to_string()), t))
            }
            (Some(name), None) => {
                notes.push(format!("profile {name} needs a config folder (HOME is not set)"));
                None
            }
            (None, _) => None,
        };
        let project_text = read(&places.project.join(".jev").join("config.env"));
        let mut layers: Vec<(Source, &str)> = Vec::new();
        if let Some(t) = &user_text {
            layers.push((Source::User, t));
        }
        if let Some((source, t)) = &profile_text {
            layers.push((source.clone(), t));
        }
        if let Some(t) = &project_text {
            layers.push((Source::Project, t));
        }
        let mut merged = Self::merge(&layers);
        merged.problems.splice(0..0, notes);
        merged
    }

    /// Give each setting to `set` unless the process environment already has it (`env_has`).
    /// Returns how many were applied.
    pub fn apply(&self, env_has: impl Fn(&str) -> bool, mut set: impl FnMut(&str, &str)) -> usize {
        let mut applied = 0;
        for s in self.settings.iter().filter(|s| !env_has(&s.key)) {
            set(&s.key, &s.value);
            applied += 1;
        }
        applied
    }

    /// Apply to this process's environment. Call before anything reads a setting.
    pub fn apply_to_process(&self) -> usize {
        self.apply(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()), |k, v| std::env::set_var(k, v))
    }
}

/// The profile names available in `user`'s config folder, sorted.
pub fn profiles(user: &Path) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(user.join("profiles")) else { return Vec::new() };
    let mut names: Vec<String> = read
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".env")).map(str::to_string))
        .collect();
    names.sort();
    names
}

/// `/settings`: each effective setting with where it came from, then the profiles.
pub fn settings_text(layered: &Layered, active: Option<&str>, available: &[String], style: crate::render::Style) -> String {
    let mut lines = vec![style.bold("settings from files") + &style.dim("  environment variables win; keys stay in ~/.jev-router.env")];
    if layered.settings.is_empty() {
        lines.push(style.dim("  none (config.env, profiles/<name>.env, .jev/config.env)"));
    }
    let width = layered.settings.iter().map(|s| s.key.len()).max().unwrap_or(0);
    for s in &layered.settings {
        lines.push(format!("  {:<width$}  {}  {}", style.cyan(&s.key), s.value, style.dim(&format!("({})", s.source))));
    }
    let profile_row = match (active, available.is_empty()) {
        (Some(name), _) => format!("  active profile {}", style.cyan(name)),
        (None, true) => style.dim("  no profiles (profiles/<name>.env in the config folder, then --profile <name>)"),
        (None, false) => style.dim("  no profile active (--profile <name> or JEV_PROFILE)"),
    };
    lines.push(style.bold("profiles"));
    lines.push(profile_row);
    if !available.is_empty() {
        lines.push(format!("  available: {}", available.join(", ")));
    }
    for problem in &layered.problems {
        lines.push(format!("{} {problem}", style.yellow("⚠")));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Style;

    fn get<'a>(l: &'a Layered, key: &str) -> Option<(&'a str, &'a Source)> {
        l.settings.iter().find(|s| s.key == key).map(|s| (s.value.as_str(), &s.source))
    }

    #[test]
    fn a_higher_layer_replaces_a_lower_one_and_says_so() {
        let l = Layered::merge(&[
            (Source::User, "JEV_PROVIDERS=on\nJEV_BELL=off\n"),
            (Source::Profile("ci".into()), "export JEV_PROVIDERS=\"off\"\n# comment\n"),
            (Source::Project, "JEV_BELL=on"),
        ]);
        assert_eq!(get(&l, "JEV_PROVIDERS"), Some(("off", &Source::Profile("ci".into()))));
        assert_eq!(get(&l, "JEV_BELL"), Some(("on", &Source::Project)));
        assert!(l.problems.is_empty(), "{:?}", l.problems);
    }

    #[test]
    fn secrets_and_unsafe_project_names_are_refused() {
        let l = Layered::merge(&[
            (Source::User, "JEV_API_KEY=abc\nSOME_TOKEN=x\nCODE_PERMISSION_MODE=plan\n"),
            (Source::Project, "CODE_PERMISSION_MODE=auto\nJEV_DELEGATE_BIN=/tmp/evil\nJEV_BELL=off\nbroken line\n1X=2\n"),
        ]);
        assert_eq!(get(&l, "CODE_PERMISSION_MODE"), Some(("plan", &Source::User)));
        assert_eq!(get(&l, "JEV_BELL").map(|g| g.0), Some("off"));
        assert!(get(&l, "JEV_API_KEY").is_none() && get(&l, "SOME_TOKEN").is_none() && get(&l, "JEV_DELEGATE_BIN").is_none());
        let all = l.problems.join("\n");
        for needle in ["JEV_API_KEY is a secret", "SOME_TOKEN is a secret", "CODE_PERMISSION_MODE may not be set by a project", "JEV_DELEGATE_BIN may not", "not KEY=VALUE: broken line", "`1X`"] {
            assert!(all.contains(needle), "{needle} in {all}");
        }
    }

    #[test]
    fn a_project_may_turn_routing_and_orchestration_off_but_not_on() {
        let l = Layered::merge(&[(Source::User, "JEV_PROVIDERS=off\nJEV_ORCHESTRATE=off\n"), (Source::Project, "JEV_PROVIDERS=on\nJEV_ORCHESTRATE=1\n")]);
        assert_eq!(get(&l, "JEV_PROVIDERS"), Some(("off", &Source::User)));
        assert_eq!(get(&l, "JEV_ORCHESTRATE"), Some(("off", &Source::User)));
        assert!(l.problems.iter().any(|p| p.contains("may only turn JEV_PROVIDERS off")), "{:?}", l.problems);
        let l = Layered::merge(&[(Source::User, "JEV_PROVIDERS=on\n"), (Source::Project, "JEV_PROVIDERS=off\n")]);
        assert_eq!(get(&l, "JEV_PROVIDERS"), Some(("off", &Source::Project)));
    }

    #[test]
    fn the_environment_wins_when_applying() {
        let l = Layered::merge(&[(Source::User, "A_ONE=1\nA_TWO=2\n")]);
        let mut set = Vec::new();
        let n = l.apply(|k| k == "A_ONE", |k, v| set.push((k.to_string(), v.to_string())));
        assert_eq!((n, set), (1, vec![("A_TWO".to_string(), "2".to_string())]));
    }

    #[test]
    fn load_reads_the_three_files_and_reports_a_missing_profile() {
        let root = std::env::temp_dir().join(format!("jev-layers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (user, project) = (root.join("user"), root.join("project"));
        std::fs::create_dir_all(user.join("profiles")).unwrap();
        std::fs::create_dir_all(project.join(".jev")).unwrap();
        std::fs::write(user.join("config.env"), "JEV_ORCHESTRATE=on\nJEV_BELL=on\n").unwrap();
        std::fs::write(user.join("profiles/quiet.env"), "JEV_BELL=off\n").unwrap();
        std::fs::write(project.join(".jev/config.env"), "JEV_ORCHESTRATE=off\n").unwrap();
        let places = Places { project, user: Some(user.clone()) };
        let l = Layered::load(&places, Some("quiet"));
        assert_eq!(get(&l, "JEV_BELL"), Some(("off", &Source::Profile("quiet".into()))));
        assert_eq!(get(&l, "JEV_ORCHESTRATE"), Some(("off", &Source::Project)));
        assert_eq!(profiles(&user), vec!["quiet".to_string()]);
        let missing = Layered::load(&places, Some("nope"));
        assert!(missing.problems[0].contains("no profile named nope"), "{:?}", missing.problems);
        assert_eq!(get(&missing, "JEV_BELL"), Some(("on", &Source::User)));
        assert!(Layered::load(&places, Some("../x")).problems[0].contains("not a usable name"));
    }

    #[test]
    fn the_settings_screen_lists_values_sources_and_profiles() {
        let l = Layered::merge(&[(Source::Profile("ci".into()), "JEV_PROVIDERS=off\nPASSWORD=x")]);
        let text = settings_text(&l, Some("ci"), &["ci".into(), "work".into()], Style::off());
        assert!(text.contains("JEV_PROVIDERS  off  (profile ci)") && text.contains("active profile ci") && text.contains("available: ci, work"), "{text}");
        assert!(text.contains("⚠ profile ci: PASSWORD is a secret"), "{text}");
        assert!(settings_text(&Layered::default(), None, &[], Style::off()).contains("no profiles"));
    }
}
