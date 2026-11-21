//! Reading the privacy policy files from disk (the only IO besides the audit log).

use std::path::{Path, PathBuf};

use super::{Policy, PolicyError, Source};

const RULES_FILE: &str = "privacy.rules";

impl Policy {
    /// The policy files, user's first: `<state>/privacy.rules`, then `<project>/.jev/privacy.rules`.
    /// A missing file is an empty policy; an unreadable file forces local-only mode.
    pub fn load(state_dir: Option<&Path>, project: &Path) -> (Self, Vec<(PathBuf, PolicyError)>) {
        let mut policy = Self::default();
        let mut errors = Vec::new();
        let files = [
            (state_dir.map(|d| d.join(RULES_FILE)), Source::User),
            (Some(project.join(".jev").join(RULES_FILE)), Source::Project),
        ];
        for (path, source) in files {
            let Some(path) = path else { continue };
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                // A folder on the way that is a file means there is no rules file either.
                Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => continue,
                Err(error) => {
                    policy = policy.with_local_only(true);
                    errors.push((path, PolicyError { line: 0, message: format!("could not read privacy rules: {error}; local-only mode is on") }));
                    continue;
                }
            };
            let (loaded, errs) = Self::parse(&text, source);
            errors.extend(errs.into_iter().map(|e| (path.clone(), e)));
            policy = policy.merged(loaded);
        }
        (policy, errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jev-privacy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn policy_files_load_from_state_and_project() {
        let state = temp("state");
        let project = temp("project");
        std::fs::create_dir_all(project.join(".jev")).unwrap();
        std::fs::write(state.join("privacy.rules"), "redact off\nblock a/** from grok\n").unwrap();
        std::fs::write(project.join(".jev/privacy.rules"), "local-only on\nredact off\n").unwrap();
        let (p, errors) = Policy::load(Some(&state), &project);
        assert!(p.local_only());
        assert_eq!(p.rule_count(), 1);
        assert_eq!(errors.len(), 1, "the project's `redact off` is refused");
        assert!(errors[0].0.ends_with(".jev/privacy.rules"));
        // Neither file: the default.
        let (none, errs) = Policy::load(None, &temp("empty"));
        assert_eq!(none, Policy::default());
        assert!(errs.is_empty());
    }

    #[test]
    fn unreadable_policy_files_force_locked_local_only_and_report_the_problem() {
        let state = temp("unreadable-state");
        let project = temp("unreadable-project");
        let file = state.join("privacy.rules");
        std::fs::write(&file, [0xff, 0xfe]).unwrap();
        let (mut p, errors) = Policy::load(Some(&state), &project);
        assert!(p.local_only());
        assert!(p.set_local_only(false).is_err());
        assert!(crate::shared::ALL_PROVIDERS.iter().all(|provider| !p.check(Path::new("/work/app"), "hello", &[]).permits(*provider)));
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, file);
        assert!(errors[0].1.to_string().contains("could not read privacy rules"));
        assert!(errors[0].1.to_string().contains("local-only"));

        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let (p, errors) = Policy::load(Some(&state), &project);
        assert!(p.local_only());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, file);
    }
}
