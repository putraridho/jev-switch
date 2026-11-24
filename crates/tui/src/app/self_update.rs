//! `/update` and `jev-code update`: bring the installed jev-code up to the newest commit of its
//! source checkout's upstream branch (`origin/main`), and reinstall it.
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
    let (dir, named) = match jev_shared::infrastructure::env_file::lookup("JEV_SOURCE_DIR").filter(|d| !d.trim().is_empty()) {
        Some(d) => (PathBuf::from(d), true),
        None => (std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), std::path::Path::to_path_buf), false),
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
    /// `origin/main`, or `None` when the branch has no upstream (or HEAD is detached).
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
        return Plan::Refused("the checkout's branch has no upstream to update from (or HEAD is detached); check out main".into());
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
/// names ([`jev_shared::infrastructure::git_guard::no_repo_programs`]).
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    git_env(dir, args, &[])
}

/// [`git`] with extra environment variables for the one command.
fn git_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(jev_shared::infrastructure::git_guard::no_repo_programs(dir))
        .args(args)
        .envs(env.iter().copied())
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(git_failure(args, &out))
    }
}

/// Preserve diagnostics from git and its transports, including helpers that write to stdout
/// or exit without a message. Clean external output before it reaches the terminal.
fn git_failure(args: &[&str], out: &std::process::Output) -> String {
    let diagnostics = [&out.stderr, &out.stdout]
        .into_iter()
        .map(|bytes| crate::render::clean_text(&String::from_utf8_lossy(bytes)))
        .filter(|text| !text.trim().is_empty())
        .map(|text| text.trim().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let status = format!("git {} failed ({})", args.join(" "), out.status);
    if diagnostics.is_empty() { status } else { format!("{status}: {diagnostics}") }
}

/// Fetch the upstream and read the facts. The only network call; it changes no file of the
/// working tree.
pub fn check(dir: &Path) -> Result<Facts, String> {
    check_with(dir, &[])
}

/// [`check`] with extra environment for the fetch (the background check keeps ssh from asking
/// for a passphrase on the terminal the session owns).
pub fn check_with(dir: &Path, fetch_env: &[(&str, &str)]) -> Result<Facts, String> {
    let upstream = git(dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let changed: Vec<String> =
        git(dir, &["status", "--porcelain", "--untracked-files=no"])?.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    let local_version = std::fs::read_to_string(dir.join("Cargo.toml")).ok().as_deref().and_then(cargo_version);
    let Some(up) = upstream.clone() else {
        return Ok(Facts { upstream: None, changed, ahead: 0, behind: 0, incoming: Vec::new(), upstream_version: None, local_version });
    };
    // `origin/main` → fetch `origin`. No `--tags`: the check reads the branch, not tags, and
    // `--tags` fails the whole fetch when a local tag differs from the remote one ("would
    // clobber existing tag"). A plain fetch still follows new tags on the fetched history.
    let remote = up.split_once('/').map_or(up.as_str(), |(r, _)| r);
    // Output is captured, so successful fetches remain quiet in the UI. Do not ask git to
    // suppress diagnostics: a failed transport may otherwise leave only "failed" to show.
    git_env(dir, &["fetch", remote], fetch_env)?;
    let counts = git(dir, &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])?;
    let mut parts = counts.split_whitespace().map(|n| n.parse::<usize>().unwrap_or(0));
    let (ahead, behind) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    let incoming = if behind > 0 {
        let limit = format!("-{MAX_COMMITS_SHOWN}");
        git(dir, &["log", "--oneline", "--no-decorate", &limit, "HEAD..@{upstream}"])?.lines().map(crate::render::clean_text).collect()
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
    let run = crate::app::host::shell::run_shell(&format!("./{INSTALL_SCRIPT}"), dir, on_line);
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

/// Where this jev-code gets its updates from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The git checkout it was built from: fetch, fast-forward, `scripts/install.sh`.
    Checkout(PathBuf),
    /// GitHub releases: download the tarball for this platform and replace the programs.
    Release,
}

/// A downloaded release updates from releases; a build from a checkout updates the checkout.
/// `JEV_SOURCE_DIR` names a checkout explicitly and so always wins.
pub fn source() -> Result<Source, String> {
    let named = jev_shared::infrastructure::env_file::lookup("JEV_SOURCE_DIR").is_some_and(|d| !d.trim().is_empty());
    if crate::app::release_update::is_release_build() && !named {
        Ok(Source::Release)
    } else {
        source_dir().map(Source::Checkout)
    }
}

impl Source {
    /// What is being asked, for "checking … for a new version"; `show` writes a folder.
    pub fn label(&self, show: impl Fn(&Path) -> String) -> String {
        match self {
            Source::Checkout(dir) => show(dir),
            Source::Release => format!("github.com/{} releases", crate::app::release_update::REPO),
        }
    }
}

/// What carrying out an update does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Checkout { dir: PathBuf, pull: bool },
    Release(crate::app::release_update::Release),
}

impl Action {
    /// Do it, reporting progress line by line; the version now installed.
    pub fn run(&self, on_line: &mut dyn FnMut(&str)) -> Result<String, String> {
        match self {
            Action::Checkout { dir, pull } => apply(dir, *pull, on_line),
            Action::Release(release) => crate::app::release_update::apply(release, on_line),
        }
    }
}

/// What `/update` found, ready to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prepared {
    /// The running program is the newest.
    UpToDate(Vec<String>),
    /// It cannot be done; the lines say why.
    Refused(Vec<String>),
    /// There is something newer: show `lines`, ask `question`, and on a yes run `action`.
    Offer { lines: Vec<String>, question: &'static str, action: Action },
}

/// What installing a release does besides replacing the programs, said before the question.
pub const RELEASE_NOTE: &str = "no build is needed; installing stops a running jev-gateway, so an open jev-claude needs a restart";

/// Look at the source (the network, never a file of the install) and decide.
pub fn prepare(source: &Source) -> Prepared {
    match source {
        Source::Checkout(dir) => {
            let plan = match check(dir) {
                Ok(facts) => plan(&facts, running_version()),
                Err(why) => Plan::Refused(why),
            };
            let lines = plan_text(&plan);
            match plan {
                Plan::UpToDate { .. } => Prepared::UpToDate(lines),
                Plan::Refused(_) => Prepared::Refused(lines),
                Plan::Update { pull, .. } => Prepared::Offer {
                    lines,
                    question: if pull { "Pull it and reinstall jev-code now? [y/N] " } else { "Reinstall jev-code from the checkout now? [y/N] " },
                    action: Action::Checkout { dir: dir.clone(), pull },
                },
            }
        }
        Source::Release => prepare_release(crate::app::release_update::latest(), running_version()),
    }
}

/// [`prepare`] for a release, pure over what the API said.
fn prepare_release(latest: Result<crate::app::release_update::Release, String>, running: &str) -> Prepared {
    let release = match latest {
        Ok(release) => release,
        Err(why) => return Prepared::Refused(vec![format!("cannot update: {why}")]),
    };
    if !crate::app::update_notice::is_newer(&release.version, running) {
        return Prepared::UpToDate(vec![format!("jev-code {running} is the newest version")]);
    }
    let mut lines = vec![format!("jev-code {running} → {}", release.version)];
    if !release.page.is_empty() {
        lines.push(format!("  {}", crate::render::clean_text(&release.page)));
    }
    lines.push(format!("({RELEASE_NOTE})"));
    Prepared::Offer { lines, question: "Download it and replace jev-code now? [y/N] ", action: Action::Release(release) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> crate::app::release_update::Release {
        use crate::app::release_update::{Allow, Platform, Release};
        Release {
            version: version.into(),
            tag: format!("v{version}"),
            page: "https://github.com/x/y/releases/tag/v1".into(),
            platform: Platform::LinuxX86_64,
            archive_url: "https://h/a".into(),
            checksum_url: "https://h/b".into(),
            allow: Allow::HttpsOnly,
        }
    }

    #[test]
    fn a_newer_release_is_offered_and_the_same_or_older_is_not() {
        let Prepared::Offer { lines, question, action } = prepare_release(Ok(release("1.14.0")), "1.13.3") else { panic!() };
        assert_eq!(lines[0], "jev-code 1.13.3 → 1.14.0");
        assert!(lines.last().is_some_and(|l| l.contains("jev-gateway")) && question.contains("Download"), "{lines:?}");
        assert!(matches!(action, Action::Release(r) if r.version == "1.14.0"));
        assert!(matches!(prepare_release(Ok(release("1.13.3")), "1.13.3"), Prepared::UpToDate(_)));
        assert!(matches!(prepare_release(Ok(release("1.12.0")), "1.13.3"), Prepared::UpToDate(_)), "an older release is never an update");
        let Prepared::Refused(lines) = prepare_release(Err("offline".into()), "1.13.3") else { panic!() };
        assert_eq!(lines, vec!["cannot update: offline".to_string()]);
    }

    #[test]
    fn failed_git_reports_status_even_without_diagnostics() {
        let out = Command::new("sh").args(["-c", "exit 42"]).output().unwrap();
        let message = git_failure(&["fetch", "--tags", "origin"], &out);
        assert!(message.contains("git fetch --tags origin failed"), "{message}");
        assert!(message.contains("42"), "{message}");
    }

    #[test]
    fn failed_git_preserves_both_diagnostic_streams_without_terminal_controls() {
        let mut out = Command::new("sh").args(["-c", "exit 1"]).output().unwrap();
        out.stderr = b"fatal: Could not read from remote repository.\n".to_vec();
        out.stdout = b"\x1b[31mSSH helper failed\x1b[0m\n".to_vec();
        let message = git_failure(&["fetch"], &out);
        assert!(message.contains("fatal: Could not read from remote repository."), "{message}");
        assert!(message.contains("SSH helper failed"), "{message}");
        assert!(!message.contains('\x1b'), "{message:?}");
    }

    fn facts() -> Facts {
        Facts {
            upstream: Some("origin/main".into()),
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
            Plan::Update { from: "1.2.0".into(), to: "1.3.0".into(), pull: true, incoming: vec!["abc feat".into(), "def fix".into()], behind: 3, upstream: "origin/main".into() }
        );
        let text = plan_text(&p).join("\n");
        assert!(text.starts_with("jev-code 1.2.0 → 1.3.0: 3 new commits on origin/main"), "{text}");
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
        sh(&upstream, "chmod +x scripts/install.sh && git init -q -b main && git add -A && git -c user.email=t@t -c user.name=t commit -qm one");
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
