//! `/update` and `jev-code update`: bring the installed jev-code up to the newest commit of its
//! source checkout's upstream branch (`origin/master`), and reinstall it.
//!
//! jev-code is installed from a git checkout (`scripts/install.sh`), so updating is: fetch the
//! branch's upstream, fast-forward to it, run the install script. The checkout is the one the
//! running binary was built from (`JEV_SOURCE_DIR` overrides). Nothing is ever merged, rebased
//! or thrown away: a checkout with changes to tracked files, a detached head, no upstream or a
//! branch that has diverged from it is refused, with what to do. The person sees the incoming
//! commits and the version change, and confirms, before anything changes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The install script, relative to the checkout.
const INSTALL_SCRIPT: &str = "scripts/install.sh";

/// The most incoming commits listed; the rest are counted.
pub const MAX_COMMITS_SHOWN: usize = 15;

/// The version of the running program.
pub fn running_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The checkout to update from: `JEV_SOURCE_DIR`, else the folder this binary was built in.
/// It must hold the install script and be a git checkout.
pub fn source_dir() -> Result<PathBuf, String> {
    let (dir, named) = match crate::envfile::lookup("JEV_SOURCE_DIR").filter(|d| !d.trim().is_empty()) {
        Some(d) => (PathBuf::from(d), true),
        None => (PathBuf::from(env!("CARGO_MANIFEST_DIR")), false),
    };
    let how = if named { "JEV_SOURCE_DIR" } else { "the folder jev-code was built in" };
    if !dir.join(INSTALL_SCRIPT).is_file() || !dir.join(".git").exists() {
        return Err(format!(
            "{how} ({}) is not a jev-code checkout with {INSTALL_SCRIPT}; set JEV_SOURCE_DIR to your clone of the repository",
            dir.display()
        ));
    }
    Ok(dir)
}

/// The `version = "..."` of a Cargo.toml's `[package]`.
pub fn cargo_version(toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package {
            if let Some(value) = line.strip_prefix("version").map(str::trim_start).and_then(|r| r.strip_prefix('=')) {
                return Some(value.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// What the checkout and its upstream say, read from git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// `origin/master`, or `None` when the branch has no upstream (or HEAD is detached).
    pub upstream: Option<String>,
    /// Tracked files with changes, as `git status --porcelain` lists them.
    pub changed: Vec<String>,
    /// Commits on the branch that the upstream does not have, and the other way round.
    pub ahead: usize,
    pub behind: usize,
    /// The upstream's newest commits, newest first: `abc1234 subject`.
    pub incoming: Vec<String>,
    /// The version the upstream's Cargo.toml names.
    pub upstream_version: Option<String>,
    /// The version the checkout's own Cargo.toml names.
    pub local_version: Option<String>,
}

/// What `/update` should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Nothing newer anywhere: the running program is the newest.
    UpToDate { version: String },
    /// Fast-forward to the upstream (when `pull`) and reinstall, going from `from` to `to`.
    Update { from: String, to: String, pull: bool, incoming: Vec<String>, behind: usize, upstream: String },
    /// It cannot be done safely; the text says why and what to do.
    Refused(String),
}

/// The decision, pure over the facts and the running version.
pub fn plan(facts: &Facts, running: &str) -> Plan {
    let Some(upstream) = facts.upstream.clone() else {
        return Plan::Refused("the checkout's branch has no upstream to update from (or HEAD is detached); check out master".into());
    };
    if facts.ahead > 0 && facts.behind > 0 {
        return Plan::Refused(format!(
            "the checkout's branch and {upstream} have diverged ({} local, {} new); merge or rebase it yourself first",
            facts.ahead, facts.behind
        ));
    }
    let update = if facts.behind > 0 {
        let to = facts.upstream_version.clone().unwrap_or_else(|| "?".into());
        Plan::Update { from: running.to_string(), to, pull: true, incoming: facts.incoming.clone(), behind: facts.behind, upstream }
    } else {
        // Nothing to pull, but the checkout may be newer than what is installed (pulled by hand).
        match &facts.local_version {
            Some(local) if local != running => {
                Plan::Update { from: running.to_string(), to: local.clone(), pull: false, incoming: Vec::new(), behind: 0, upstream }
            }
            _ => return Plan::UpToDate { version: running.to_string() },
        }
    };
    // Changes to tracked files matter only when something would happen: a pull could clash
    // with them, and a reinstall would build them.
    if !facts.changed.is_empty() {
        let shown: Vec<&str> = facts.changed.iter().take(5).map(String::as_str).collect();
        return Plan::Refused(format!(
            "the checkout has uncommitted changes to tracked files ({}); commit or stash them first, nothing was changed",
            shown.join(", ")
        ));
    }
    update
}

/// `git -C dir args`, its stdout when it succeeded, else git's own words. Never asks for a
/// password (the session owns the terminal) and never runs a program the checkout's config
/// names ([`crate::gitflow::no_repo_programs`]).
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(crate::gitflow::no_repo_programs(dir))
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!("git {}: {}", args.join(" "), if said.is_empty() { "failed".to_string() } else { said }))
    }
}

/// Fetch the upstream and read the facts. The only network call; it changes no file of the
/// working tree.
pub fn check(dir: &Path) -> Result<Facts, String> {
    let upstream = git(dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let changed: Vec<String> =
        git(dir, &["status", "--porcelain", "--untracked-files=no"])?.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    let local_version = std::fs::read_to_string(dir.join("Cargo.toml")).ok().as_deref().and_then(cargo_version);
    let Some(up) = upstream.clone() else {
        return Ok(Facts { upstream: None, changed, ahead: 0, behind: 0, incoming: Vec::new(), upstream_version: None, local_version });
    };
    // `origin/master` → fetch `origin`, with its tags (a release is a tag).
    let remote = up.split_once('/').map_or(up.as_str(), |(r, _)| r);
    git(dir, &["fetch", "--quiet", "--tags", remote])?;
    let counts = git(dir, &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])?;
    let mut parts = counts.split_whitespace().map(|n| n.parse::<usize>().unwrap_or(0));
    let (ahead, behind) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    let incoming = if behind > 0 {
        let limit = format!("-{MAX_COMMITS_SHOWN}");
        git(dir, &["log", "--oneline", "--no-decorate", &limit, "HEAD..@{upstream}"])?.lines().map(crate::uifmt::clean_text).collect()
    } else {
        Vec::new()
    };
    let upstream_version = git(dir, &["show", "@{upstream}:Cargo.toml"]).ok().as_deref().and_then(cargo_version);
    Ok(Facts { upstream, changed, ahead, behind, incoming, upstream_version, local_version })
}

/// Carry out an [`Plan::Update`]: fast-forward (when `pull`), then the install script, its
/// output line by line to `on_line`. Returns the version now installed.
pub fn apply(dir: &Path, pull: bool, on_line: &mut dyn FnMut(&str)) -> Result<String, String> {
    if pull {
        // `--ff-only`: if anything moved since the check, refuse rather than merge.
        git(dir, &["merge", "--ff-only", "--quiet", "@{upstream}"])?;
    }
    let run = crate::host::run_shell(&format!("./{INSTALL_SCRIPT}"), dir, on_line);
    if let Some(failed) = run.failed {
        return Err(format!("could not run {INSTALL_SCRIPT}: {failed}"));
    }
    if run.code != Some(0) {
        let code = run.code.map_or_else(|| "a signal".to_string(), |c| format!("code {c}"));
        return Err(format!("{INSTALL_SCRIPT} failed with {code}; the checkout is updated, the installed jev-code is not (see the output above)"));
    }
    Ok(std::fs::read_to_string(dir.join("Cargo.toml")).ok().as_deref().and_then(cargo_version).unwrap_or_else(|| "?".into()))
}

/// What installing does besides replacing the programs, said before the question.
pub const INSTALL_NOTE: &str =
    "the release build takes a minute and cannot be stopped halfway; installing stops a running jev-gateway, so an open jev-claude needs a restart";

/// The lines that present a plan before asking.
pub fn plan_text(plan: &Plan) -> Vec<String> {
    let mut lines = plan_lines(plan);
    if matches!(plan, Plan::Update { .. }) {
        lines.push(format!("({INSTALL_NOTE})"));
    }
    lines
}

fn plan_lines(plan: &Plan) -> Vec<String> {
    match plan {
        Plan::UpToDate { version } => vec![format!("jev-code {version} is the newest version")],
        Plan::Refused(why) => vec![format!("cannot update: {why}")],
        Plan::Update { from, to, pull: true, incoming, behind, upstream } => {
            let version = if from == to { format!("jev-code {from} (same version, newer commits)") } else { format!("jev-code {from} → {to}") };
            let mut lines = vec![format!("{version}: {behind} new commit{} on {upstream}", if *behind == 1 { "" } else { "s" })];
            lines.extend(incoming.iter().map(|c| format!("  {c}")));
            if *behind > incoming.len() {
                lines.push(format!("  … and {} more", behind - incoming.len()));
            }
            lines
        }
        Plan::Update { from, to, pull: false, .. } => vec![format!("jev-code {from} → {to}: the checkout is newer than the installed program")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            upstream: Some("origin/master".into()),
            changed: Vec::new(),
            ahead: 0,
            behind: 0,
            incoming: Vec::new(),
            upstream_version: Some("1.2.0".into()),
            local_version: Some("1.2.0".into()),
        }
    }

    #[test]
    fn the_cargo_version_is_the_packages() {
        assert_eq!(cargo_version("[package]\nname = \"x\"\nversion = \"1.3.0\"\n[dependencies]\nversion = \"9\""), Some("1.3.0".into()));
        assert_eq!(cargo_version("[dependencies]\nversion = \"9\""), None);
        assert_eq!(cargo_version("[package]\nversion=\"2.0.0-rc.1\""), Some("2.0.0-rc.1".into()));
    }

    #[test]
    fn nothing_newer_is_up_to_date() {
        assert_eq!(plan(&facts(), "1.2.0"), Plan::UpToDate { version: "1.2.0".into() });
        // Local commits not pushed yet, same version installed: nothing to do either.
        assert_eq!(plan(&Facts { ahead: 2, ..facts() }, "1.2.0"), Plan::UpToDate { version: "1.2.0".into() });
    }

    #[test]
    fn new_commits_upstream_are_pulled_and_installed() {
        let f = Facts { behind: 3, incoming: vec!["abc feat".into(), "def fix".into()], upstream_version: Some("1.3.0".into()), ..facts() };
        let p = plan(&f, "1.2.0");
        assert_eq!(
            p,
            Plan::Update { from: "1.2.0".into(), to: "1.3.0".into(), pull: true, incoming: vec!["abc feat".into(), "def fix".into()], behind: 3, upstream: "origin/master".into() }
        );
        let text = plan_text(&p).join("\n");
        assert!(text.starts_with("jev-code 1.2.0 → 1.3.0: 3 new commits on origin/master"), "{text}");
        assert!(text.contains("  abc feat") && text.contains("… and 1 more"), "{text}");
        assert!(text.ends_with(&format!("({INSTALL_NOTE})")), "the gateway restart is said before asking: {text}");
        assert!(!plan_text(&Plan::UpToDate { version: "1.2.0".into() }).join("\n").contains("jev-gateway"));
    }

    #[test]
    fn a_checkout_newer_than_the_install_is_installed_without_pulling() {
        let p = plan(&Facts { local_version: Some("1.3.0".into()), ..facts() }, "1.2.0");
        assert!(matches!(&p, Plan::Update { pull: false, to, .. } if to == "1.3.0"), "{p:?}");
    }

    #[test]
    fn anything_that_could_lose_work_is_refused() {
        let refused = |f: Facts| matches!(plan(&f, "1.2.0"), Plan::Refused(_));
        assert!(refused(Facts { upstream: None, behind: 1, ..facts() }), "no upstream");
        assert!(refused(Facts { changed: vec!["M src/lib.rs".into()], behind: 1, ..facts() }), "local changes");
        assert!(refused(Facts { ahead: 1, behind: 1, ..facts() }), "diverged");
        assert!(refused(Facts { changed: vec!["M src/lib.rs".into()], local_version: Some("1.3.0".into()), ..facts() }), "a reinstall would build them");
        let Plan::Refused(why) = plan(&Facts { changed: vec!["M src/lib.rs".into()], behind: 1, ..facts() }, "1.2.0") else { panic!() };
        assert!(why.contains("src/lib.rs") && why.contains("nothing was changed"), "{why}");
    }

    #[test]
    fn local_changes_do_not_matter_when_there_is_nothing_to_do() {
        let f = Facts { changed: vec!["M src/lib.rs".into()], ..facts() };
        assert_eq!(plan(&f, "1.2.0"), Plan::UpToDate { version: "1.2.0".into() });
    }

    /// The real thing against a throwaway upstream: check sees the new commit, apply
    /// fast-forwards and runs the (fake) install script.
    #[test]
    fn check_and_apply_against_a_local_upstream() {
        let root = std::env::temp_dir().join(format!("jev-selfupdate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (upstream, clone) = (root.join("upstream"), root.join("clone"));
        std::fs::create_dir_all(upstream.join("scripts")).unwrap();
        let sh = |dir: &Path, cmd: &str| {
            let ok = Command::new("sh").arg("-c").arg(cmd).current_dir(dir).env("GIT_CONFIG_GLOBAL", "/dev/null").stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success();
            assert!(ok, "{cmd}");
        };
        std::fs::write(upstream.join("Cargo.toml"), "[package]\nversion = \"1.0.0\"\n").unwrap();
        std::fs::write(upstream.join(INSTALL_SCRIPT), "#!/bin/sh\necho installing\ntouch installed\n").unwrap();
        sh(&upstream, "chmod +x scripts/install.sh && git init -q -b master && git add -A && git -c user.email=t@t -c user.name=t commit -qm one");
        sh(&root, "git clone -q upstream clone");
        std::fs::write(upstream.join("Cargo.toml"), "[package]\nversion = \"1.1.0\"\n").unwrap();
        sh(&upstream, "git -c user.email=t@t -c user.name=t commit -qam 'bump to 1.1.0'");

        let f = check(&clone).unwrap();
        assert_eq!((f.behind, f.upstream_version.as_deref()), (1, Some("1.1.0")));
        assert!(f.incoming[0].ends_with("bump to 1.1.0"), "{:?}", f.incoming);
        let Plan::Update { pull, .. } = plan(&f, "1.0.0") else { panic!("{:?}", plan(&f, "1.0.0")) };
        let mut lines = Vec::new();
        assert_eq!(apply(&clone, pull, &mut |l| lines.push(l.to_string())), Ok("1.1.0".into()));
        assert!(lines.contains(&"installing".to_string()) && clone.join("installed").exists(), "{lines:?}");
        assert_eq!(plan(&check(&clone).unwrap(), "1.1.0"), Plan::UpToDate { version: "1.1.0".into() });

        // A tracked change in the checkout stops it before anything moves.
        std::fs::write(clone.join("Cargo.toml"), "[package]\nversion = \"9\"\n").unwrap();
        assert!(matches!(plan(&check(&clone).unwrap(), "1.1.0"), Plan::Refused(_)));
        let _ = std::fs::remove_dir_all(&root);
    }
}
