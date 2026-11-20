//! The worker side of cross-provider orchestration: a planner model calls a
//! `delegate` tool with a brief, and a cheaper WORKER model, possibly from another
//! provider, does that piece in its own git worktree. The planner then reviews the
//! returned diff and decides what to merge.
//!
//! **Jev makes every model decision.** Nothing here takes a model from the caller: the
//! tool has no model parameter and [`DelegateRequest`] has no model field, so anything a
//! planner says about models cannot reach this code. Code only
//! 1. filters to the models that are really usable (logged in, not cooling down, billing
//!    allowed: [`crate::providers::application::provider_registry::Runtime`], [`crate::catalog::domain::model_table::Usable`]),
//! 2. applies hard safety rules ([`pick_worker`]: never Top tier, never Fable/Astra/
//!    Carbon, security work never to DeepSeek/Grok, code-writing work never below
//!    Everyday), and
//! 3. maps Jev's judgment of the brief (its task type, via the catalog's chains) to the
//!    first model that passes.
//!
//! A worktree starts from `HEAD`, so the planner's UNCOMMITTED edits are invisible to
//! workers: the tool description tells the planner to commit what a worker needs or put
//! it in the brief.
//!
//! Layout: [`pick_worker`] and the text helpers are pure; [`Worktrees`] is the only git
//! door ([`GitWorktrees`] real, tests fake it); [`Delegator`] sequences judge, pick,
//! worktree, run, retry, log; [`DelegateApi`] is what the MCP server sees.
//!
//! **Where a worker runs and how its work lands** ([`Landing`]). A read-only worker runs in
//! the user's own folder in plan mode, when its provider enforces plan mode
//! ([`plan_mode_enforced`]), and returns only a report; on a provider that does not, it runs
//! in a worktree that is thrown away after. An editing worker gets its own worktree; when it
//! finishes, jev runs the project's tests there ([`crate::orchestration::infrastructure::worker_check`]) and, if they pass and
//! the changes apply cleanly, merges them itself: the planner gets the report and the file
//! list, not the diff, and `undo` takes them back out. When there is no test command, the
//! tests fail, the patch conflicts, or the planner asked to review, it falls back to review.
//!
//! **Delegations run in the background.** [`Delegator::start`] returns a task id at once and
//! runs the worker (and its tests) on its own task; [`Delegator::wait`] collects it and only
//! then lands it ([`Delegator::settle`]), so nothing reaches the main tree while the planner
//! may be editing it. [`Delegator::delegate`] does both in one blocking call.
//!
//! **In review, the planner merges only what it saw.** The delegate result lists every changed file
//! with its +/- counts and carries whole-file diffs up to a size budget; `show_diff` pages
//! the rest. The registry records which files were shown in full and `merge` refuses any
//! other. The registry is saved to `<state>/workers` so a restarted server still knows
//! its workers, and [`Delegator::recover`] removes abandoned checkouts. Git calls and
//! record IO are blocking, so the delegator runs them on the blocking pool
//! ([`Delegator::off_thread`]) and never under the registry lock.
//!
//! **Servers share the state directory.** Every record carries its owner (pid plus process
//! start time, [`Liveness`]); `recover` never touches a record whose owner is alive, and
//! adopts or ages out only a dead server's. Worker ids carry a per-server tag, so two
//! servers never produce the same id. A delegation dropped mid-flight (a blocking call
//! cancelled, or a background task dropped when the server shuts down) removes its
//! placeholder, record and worktree ([`Reservation`]). A path read from a record is
//! deleted only if it proves to be inside the worktrees root ([`checked_inside`]).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::backend::{self, Event, ProviderId, TurnError, TurnOutcome, TurnRequest};
use crate::catalog::domain::model_table::{allowed_for, chain_with, models, pick, Allowances, ModelSpec, Overrides, TaskType, Tier, Usable};
use crate::shared::infrastructure::jsonl_log as log;
use crate::approval::domain::mode::Mode;
use crate::providers::application::provider_registry::Runtime;
use crate::judgment::application::judge::{ProviderWant, Router, CODE_EDIT_P, CODE_SECURITY_P};
use crate::backend::{idle_timeout_from_env, BackendFactory};
use crate::backend::infrastructure::outcome_wire::turn_row;
use crate::orchestration::infrastructure::worker_check::{CheckRun, Checks, ProjectChecks};

/// A worker that fails in a way another provider can fix is retried this many times in
/// all (so: once more), on the next worker pick.
pub const MAX_WORKER_ATTEMPTS: u32 = 2;
/// Most characters of diff handed back at once, in the delegate result and per `show_diff` page.
pub const MAX_DIFF_CHARS: usize = 20_000;
/// Most characters of the worker's final text handed back.
pub const MAX_SUMMARY_CHARS: usize = 4_000;
/// A provider whose plan is this much busier than the least busy candidate is passed
/// over for the next in Jev's chain: within the margin chain order wins (it is Jev's
/// quality judgment), beyond it the work is spread to protect the busy plan.
const SPREAD_SLACK: f64 = 0.10;
/// How much one worker already running counts as plan usage when spreading, so parallel
/// workers fan out across providers even before any provider reports its usage.
const LIVE_WORKER_LOAD: f64 = 0.15;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// --------------------------------------------------------------- picking a worker

/// What Jev said about a brief, reduced to what choosing a worker needs.
#[derive(Debug, Clone, PartialEq)]
pub struct BriefSignals {
    pub task: TaskType,
    /// P(the brief is about security), independent of keywords.
    pub security_p: f64,
    /// P(the brief changes files or runs commands).
    pub edit_p: f64,
    /// The planner says no files change. Only a hint: it lowers the floor to Light only
    /// when Jev also says the brief is not an edit.
    pub read_only: bool,
}

impl BriefSignals {
    pub fn from_want(w: &ProviderWant, read_only: bool) -> Self {
        Self { task: w.task, security_p: w.security_p, edit_p: w.edit_p, read_only }
    }

    /// Jev was unavailable. A brief nobody judged could be security work, which has hard
    /// vendor rules, so it is treated as security (never DeepSeek/Grok) and as an edit.
    pub fn unjudged() -> Self {
        Self { task: TaskType::Security, security_p: 1.0, edit_p: 1.0, read_only: false }
    }
}

/// The task type that picks the chain. Security is a hard rule that overrides Jev's task
/// label, like in routing, but judged by Jev's `security_p` only: the keyword lists
/// are not used for workers because a brief is full of incidental words ("no unsafe
/// code") that tagged every worker security-sensitive.
pub fn worker_task(s: &BriefSignals) -> TaskType {
    if s.task == TaskType::Security || s.security_p >= CODE_SECURITY_P {
        TaskType::Security
    } else {
        s.task
    }
}

/// The lowest tier this worker may run on. Work that writes code is never below
/// Everyday; Light is for lookups the planner calls read-only AND Jev agrees do not edit.
pub fn worker_floor(s: &BriefSignals) -> Tier {
    if s.read_only && s.edit_p < CODE_EDIT_P {
        Tier::Light
    } else {
        Tier::Everyday
    }
}

/// Workers are never Top tier (that is the planner's), and never the models kept for
/// very hard work (Fable, Astra, Carbon) whatever their tier says.
fn worker_eligible(spec: &ModelSpec, floor: Tier) -> bool {
    spec.tier != Tier::Top && !spec.very_hard_only && spec.tier >= floor
}

/// Why no worker can take a brief; the caller says so instead of guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoWorker {
    pub task: TaskType,
    pub floor: Tier,
}

impl std::fmt::Display for NoWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let security = if self.task == TaskType::Security { " (security work never goes to deepseek or grok)" } else { "" };
        write!(
            f,
            "no usable worker model for {} work at {:?} tier or above{security}; workers are never top-tier models. \
             Do this piece yourself.",
            self.task, self.floor
        )
    }
}

impl std::error::Error for NoWorker {}

/// Everything [`pick_worker`] reads about the world; the caller owns the IO.
pub struct WorkerView<'a> {
    pub usable: &'a Usable,
    pub allow: &'a Allowances,
    pub overrides: &'a Overrides,
    /// Fraction of the plan used, as the provider last reported it.
    pub plan_utilization: &'a dyn Fn(ProviderId) -> Option<f64>,
    /// Workers currently running on the provider.
    pub live: &'a dyn Fn(ProviderId) -> usize,
    /// Rough size of the brief, so a model whose window is too small is skipped.
    pub context_tokens: u64,
}

/// Models Jev's chain for the task lists that may be workers, then every other eligible
/// model, strongest tier first. The tail is only used when the chain has no usable
/// worker (the Security and Review chains are all Top tier).
fn candidate_lists(task: TaskType, floor: Tier, overrides: &Overrides) -> [Vec<&'static ModelSpec>; 2] {
    let ok = |s: &&'static ModelSpec| worker_eligible(s, floor) && allowed_for(task, s);
    let chain: Vec<_> = chain_with(task, false, overrides).into_iter().filter(ok).collect();
    let mut rest: Vec<_> = models().iter().filter(ok).collect();
    rest.sort_by_key(|s| std::cmp::Reverse(s.tier));
    [chain, rest]
}

/// Among serving models (best first), one per provider at its best, prefer the least
/// busy provider; but a provider within [`SPREAD_SLACK`] of the least busy keeps its
/// chain position.
fn spread(serving: &[&'static ModelSpec], view: &WorkerView) -> &'static ModelSpec {
    let mut firsts: Vec<&'static ModelSpec> = Vec::new();
    for s in serving {
        if !firsts.iter().any(|f| f.provider == s.provider) {
            firsts.push(s);
        }
    }
    let load = |s: &ModelSpec| (view.plan_utilization)(s.provider).unwrap_or(0.0) + LIVE_WORKER_LOAD * (view.live)(s.provider) as f64;
    let least = firsts.iter().map(|s| load(s)).fold(f64::INFINITY, f64::min);
    firsts.iter().copied().find(|s| load(s) <= least + SPREAD_SLACK).unwrap_or(firsts[0])
}

/// The worker model for a brief. The only inputs are Jev's signals and the live view of
/// what is usable: there is no way to ask for a particular model.
pub fn pick_worker(signals: &BriefSignals, view: &WorkerView) -> Result<&'static ModelSpec, NoWorker> {
    let task = worker_task(signals);
    let floor = worker_floor(signals);
    for list in candidate_lists(task, floor, view.overrides) {
        let serving: Vec<_> = list
            .into_iter()
            .filter(|s| pick(&[*s], task, view.usable, view.allow, view.context_tokens).is_some())
            .collect();
        if !serving.is_empty() {
            return Ok(spread(&serving, view));
        }
    }
    Err(NoWorker { task, floor })
}

// --------------------------------------------------------------- text helpers

/// The old and new path in the rest of a `diff --git a/<old> b/<new>` line (after `a/`).
/// Without a rename both halves are the same text, which is checked first so a path that
/// itself contains ` b/` is still read right.
fn header_paths(rest: &str) -> (&str, &str) {
    let n = rest.len();
    if n >= 3 && (n - 3).is_multiple_of(2) {
        let half = (n - 3) / 2;
        if rest.is_char_boundary(half) && rest.is_char_boundary(half + 3) && &rest[half..half + 3] == " b/" && rest[..half] == rest[half + 3..] {
            return (&rest[..half], &rest[..half]);
        }
    }
    rest.split_once(" b/").unwrap_or((rest, rest))
}

/// The paths a unified diff touches (both sides of a rename), in order, once each.
pub fn patch_paths(patch: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in patch.lines() {
        let Some(rest) = line.strip_prefix("diff --git a/") else { continue };
        let (old, new) = header_paths(rest);
        for p in [old, new] {
            if !out.iter().any(|o| o == p) {
                out.push(p.to_string());
            }
        }
    }
    out
}

/// One file's part of a patch, exactly as git wrote it (header, hunks, binary data).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// The path after the change (the new name of a rename). A header git quoted, which
    /// this cannot parse, keeps its raw text: the file stays listed and cannot slip through.
    pub path: String,
    pub text: String,
}

/// A patch cut into per-file parts, in order. Joining the parts gives the patch back.
pub fn split_patch(patch: &str) -> Vec<FileDiff> {
    let mut out: Vec<FileDiff> = Vec::new();
    for line in patch.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let rest = rest.trim_end();
            let path = rest.strip_prefix("a/").map_or(rest, |r| header_paths(r).1);
            out.push(FileDiff { path: path.to_string(), text: String::new() });
        }
        if let Some(cur) = out.last_mut() {
            cur.text.push_str(line);
        }
    }
    out
}

fn join_diffs<'a>(parts: impl IntoIterator<Item = &'a FileDiff>) -> String {
    parts.into_iter().map(|p| p.text.as_str()).collect()
}

/// How much of one file changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub binary: bool,
}

impl FileStat {
    pub fn render(&self) -> String {
        if self.binary {
            format!("{} (binary)", self.path)
        } else {
            format!("{} +{} -{}", self.path, self.added, self.removed)
        }
    }
}

impl FileDiff {
    pub fn stat(&self) -> FileStat {
        let (mut added, mut removed, mut binary, mut in_hunk) = (0, 0, false, false);
        for line in self.text.lines() {
            if line.starts_with("@@") {
                in_hunk = true;
            } else if line.starts_with("GIT binary patch") || line.starts_with("Binary files ") {
                binary = true;
            } else if in_hunk {
                match line.as_bytes().first() {
                    Some(b'+') => added += 1,
                    Some(b'-') => removed += 1,
                    _ => {}
                }
            }
        }
        FileStat { path: self.path.clone(), added, removed, binary }
    }

    fn chars(&self) -> usize {
        self.text.chars().count()
    }
}

/// The whole files that fit in `budget` characters, in order (one that does not fit is
/// skipped, a smaller later one may still go in), as text and as the paths shown.
fn first_page(parts: &[FileDiff], budget: usize) -> (String, Vec<String>) {
    let (mut left, mut text, mut shown) = (budget, String::new(), Vec::new());
    for p in parts {
        let n = p.chars();
        if n <= left {
            left -= n;
            text.push_str(&p.text);
            shown.push(p.path.clone());
        }
    }
    (text, shown)
}

/// `max` characters of `text` from character `offset`, and the offset just after them.
fn char_page(text: &str, offset: usize, max: usize) -> (&str, usize) {
    let byte = |chars: usize| text.char_indices().nth(chars).map_or(text.len(), |(b, _)| b);
    let (from, to) = (byte(offset), byte(offset.saturating_add(max)));
    (&text[from..to], offset + text[from..to].chars().count())
}

/// `text` cut to `max` characters, with a note of how much was dropped.
pub fn truncate_with_note(text: &str, max: usize, what: &str) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}\n[{what} truncated: {} more characters not shown]", total - max)
}

/// What a worker is told before the brief: where it may work and how to report.
pub fn worker_prompt(brief: &str, files: &[String], read_only: bool) -> String {
    let hints = if files.is_empty() {
        String::new()
    } else {
        format!("\n\nFiles that are probably involved (hints, not limits): {}", files.join(", "))
    };
    let rules = if read_only {
        "You are a worker on one READ-ONLY piece of a larger task. Rules: read and investigate inside the current \
         directory only; do not create, change or delete any file, and do not run commands that change the project; do \
         not use the network. When you are done, reply with a report of what you found, with file paths and line \
         numbers where they help."
    } else {
        "You are a worker on one piece of a larger task. Rules: work only inside the current directory (your own git \
         worktree) and do not read or write outside it; do not use the network; do not commit or touch git branches. \
         Run the project's tests before you finish: your work is merged only if they pass. When you are done, reply \
         with a short report: what you changed, and anything you could not do."
    };
    format!("{rules}\n\nTask:\n{brief}{hints}")
}

/// Whether the provider's CLI is held to plan mode by the CLI itself, so a read-only worker
/// can run in the user's own folder. Claude Code (and DeepSeek, which runs on it) has a real
/// plan mode and Codex a read-only sandbox; Gemini and Grok only fall back to their default
/// approval mode, which is not a guarantee, so their read-only workers get a worktree.
pub fn plan_mode_enforced(provider: ProviderId) -> bool {
    match provider {
        ProviderId::Claude | ProviderId::Codex | ProviderId::DeepSeek => true,
        ProviderId::Gemini | ProviderId::Grok => false,
    }
}

// --------------------------------------------------------------- worktrees

/// The only door to git. Real: [`GitWorktrees`]; tests use a fake. Every method blocks;
/// the [`Delegator`] runs them off the async threads in one place.
pub trait Worktrees: Send + Sync {
    /// The repository workers start from and merges apply to.
    fn repo(&self) -> PathBuf;
    /// The directory every worktree lives under.
    fn root(&self) -> PathBuf;
    /// A fresh isolated working copy at the main tree's `HEAD`, named `id`.
    fn create(&self, id: &str) -> Result<PathBuf>;
    /// Everything the worker changed in `path` (new files included) as one patch.
    fn diff(&self, path: &Path) -> Result<String>;
    /// Apply `patch` to the MAIN working tree. On failure the tree is left as it was.
    fn apply(&self, patch: &str) -> Result<()>;
    /// Take an applied `patch` back out of the MAIN working tree. On failure nothing changes.
    fn unapply(&self, patch: &str) -> Result<()>;
    /// Delete a worktree checkout.
    fn remove(&self, path: &Path) -> Result<()>;
    /// Forget the bookkeeping of worktrees whose directory is gone (`git worktree prune`).
    fn prune(&self, repo: &Path) -> Result<()>;
}

/// Run `git -C dir args`, optionally feeding `stdin`; stdout on success, a message with
/// git's own words on failure.
fn git(dir: &Path, args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(crate::shared::infrastructure::git_guard::no_repo_programs(dir))
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run git (is it installed?)")?;
    // A separate thread, so a big patch cannot deadlock against git printing before it has read it all.
    let feeder = stdin.zip(child.stdin.take()).map(|(text, mut pipe)| {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        })
    });
    let out = child.wait_with_output().context("waiting for git")?;
    if let Some(f) = feeder {
        let _ = f.join();
    }
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("git {} failed: {}", args.first().unwrap_or(&""), err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Files and the index as they were, so a failed merge can put the tree back exactly.
/// `git apply --3way` writes conflict markers into the working tree and stages partial
/// results when it fails, which is not "leave the tree untouched".
struct Snapshot {
    index: Option<(PathBuf, Vec<u8>)>,
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl Snapshot {
    fn take(repo: &Path, index: Option<PathBuf>, paths: &[String]) -> Self {
        let safe = |p: &&String| Path::new(p).components().all(|c| matches!(c, Component::Normal(_)));
        let files = paths.iter().filter(safe).map(|p| repo.join(p)).map(|p| {
            let bytes = std::fs::read(&p).ok();
            (p, bytes)
        });
        Self {
            index: index.and_then(|p| std::fs::read(&p).ok().map(|b| (p, b))),
            files: files.collect(),
        }
    }

    fn restore(&self) {
        for (path, bytes) in &self.files {
            match bytes {
                Some(b) => {
                    let _ = std::fs::write(path, b);
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        if let Some((path, bytes)) = &self.index {
            let _ = std::fs::write(path, bytes);
        }
    }
}

/// Why a patch did not go on, from git's words for the plain apply and the 3-way one.
fn explain_apply_failure(plain: &str, three_way: &str) -> String {
    let cause = if three_way.contains("does not match index") {
        "your working tree has uncommitted edits to a file the worker also changed, and they overlap its hunks \
         (a 3-way merge also needs those files unmodified or staged). Commit, stage or stash them, or merge by hand"
    } else if plain.contains("already exists") || three_way.contains("already exists") {
        "a file the worker created already exists in your tree"
    } else {
        "the worker's changes do not apply cleanly: they conflict with your tree"
    };
    format!("{cause}; nothing was changed. Plain apply: {plain} 3-way apply: {three_way}")
}

/// `path` as a canonical location strictly inside `root`, or why it is not one. `path`
/// may be untrusted (it was read from a saved record), so: it must be absolute and free of
/// `..`; its parent is canonicalized (the entry itself may be gone) so a symlinked
/// directory on the way cannot lead outside; the entry may not itself be a symlink; and it
/// may not be `root` itself.
fn checked_inside(root: &Path, path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("refusing to touch {}: not an absolute path without `..`", path.display());
    }
    let root = root.canonicalize().with_context(|| format!("worktrees root {} is not usable", root.display()))?;
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        bail!("refusing to touch {}: it has no parent directory and name", path.display());
    };
    let Ok(parent) = parent.canonicalize() else {
        bail!("refusing to touch {}: its parent directory cannot be resolved", path.display());
    };
    let candidate = parent.join(name);
    if candidate == root || !candidate.starts_with(&root) {
        bail!("refusing to touch {}: not strictly inside {}", path.display(), root.display());
    }
    if std::fs::symlink_metadata(&candidate).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("refusing to touch {}: it is a symlink", path.display());
    }
    Ok(candidate)
}

/// Real worktrees: `git worktree add --detach <root>/<id> HEAD`. Detached, so no branch
/// is created and nothing is left behind once the worktree is removed.
pub struct GitWorktrees {
    /// The top level of the repository: workers start there, and merges apply there.
    repo: PathBuf,
    root: PathBuf,
}

impl GitWorktrees {
    /// `dir` is any directory inside the repository; worktrees go under `root`
    /// (`<state dir>/worktrees`).
    pub fn new(dir: &Path, root: PathBuf) -> Result<Self> {
        let top = git(dir, &["rev-parse", "--show-toplevel"], None)
            .with_context(|| format!("{} is not inside a git repository, so workers cannot get worktrees", dir.display()))?;
        Ok(Self { repo: PathBuf::from(top.trim()), root })
    }

    fn index_path(&self) -> Option<PathBuf> {
        let rel = git(&self.repo, &["rev-parse", "--git-path", "index"], None).ok()?;
        Some(self.repo.join(rel.trim()))
    }

    /// A plain `git apply` first: it works on the working tree whatever is unstaged, as
    /// long as the hunks fit. Only when it does not fit, `--3way`, which also merges into
    /// text the planner changed but needs the index to match the files. On failure the
    /// files and index are put back exactly.
    fn apply_patch(&self, patch: &str, direction: Direction) -> Result<()> {
        let run = |extra: &[&str]| {
            let args: Vec<&str> = ["apply"].into_iter().chain(direction.flag()).chain(extra.iter().copied()).chain(["-"]).collect();
            git(&self.repo, &args, Some(patch))
        };
        let snapshot = Snapshot::take(&self.repo, self.index_path(), &patch_paths(patch));
        let Err(plain) = run(&["--check"]).and_then(|_| run(&[])) else { return Ok(()) };
        match run(&["--3way"]) {
            Ok(_) => Ok(()),
            Err(three_way) => {
                snapshot.restore();
                Err(anyhow!("{}", explain_apply_failure(&format!("{plain:#}."), &format!("{three_way:#}"))))
            }
        }
    }
}

/// Which way a patch goes onto the main tree: a merge, or the undo of one.
#[derive(Debug, Clone, Copy)]
enum Direction {
    Forward,
    Reverse,
}

impl Direction {
    fn flag(self) -> Option<&'static str> {
        match self {
            Self::Forward => None,
            Self::Reverse => Some("-R"),
        }
    }
}

impl Worktrees for GitWorktrees {
    fn repo(&self) -> PathBuf {
        self.repo.clone()
    }

    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn create(&self, id: &str) -> Result<PathBuf> {
        let path = self.root.join(id);
        std::fs::create_dir_all(&self.root).with_context(|| format!("create {}", self.root.display()))?;
        git(&self.repo, &["worktree", "add", "--detach", &path.to_string_lossy(), "HEAD"], None)?;
        Ok(path)
    }

    /// `git add -A` first so files the worker created show up; `--binary` so the patch
    /// applies for binary files too.
    fn diff(&self, path: &Path) -> Result<String> {
        git(path, &["add", "-A"], None)?;
        git(path, &["diff", "--cached", "--binary", "--no-ext-diff", "--no-textconv", "HEAD"], None)
    }

    fn apply(&self, patch: &str) -> Result<()> {
        self.apply_patch(patch, Direction::Forward)
    }

    /// The same as a merge, in reverse: a 3-way merge's hunks carry the planner's text as
    /// context, so taking them back out needs the 3-way step too.
    fn unapply(&self, patch: &str) -> Result<()> {
        self.apply_patch(patch, Direction::Reverse).context("the merged changes cannot be taken back out cleanly")
    }

    /// `git worktree remove`, and when git will not (the checkout belongs to another
    /// repository, or git's record of it is gone) the directory itself. The path comes from
    /// a saved record, so it is trusted only after [`checked_inside`] proves it is a real
    /// entry strictly inside `root`; nothing else is ever deleted or handed to git.
    fn remove(&self, path: &Path) -> Result<()> {
        let inside = checked_inside(&self.root, path)?;
        match git(&self.repo, &["worktree", "remove", "--force", &path.to_string_lossy()], None) {
            Ok(_) => Ok(()),
            Err(_) if !inside.exists() => Ok(()),
            Err(_) => std::fs::remove_dir_all(&inside).with_context(|| format!("remove {}", path.display())),
        }
    }

    fn prune(&self, repo: &Path) -> Result<()> {
        git(repo, &["worktree", "prune"], None).map(|_| ())
    }
}

// --------------------------------------------------------------- the registry

/// A worker started by this server and not yet merged or discarded.
#[derive(Debug, Clone)]
struct Worker {
    provider: ProviderId,
    model: String,
    /// The repository the worker was cut from.
    repo: PathBuf,
    created_ms: u64,
    /// `None` while the slot is reserved and the worktree is still being made.
    path: Option<PathBuf>,
    /// Still running; a finished worker awaiting merge or discard is not load.
    running: bool,
    /// Files the planner has seen IN FULL: only these may be merged.
    shown: BTreeSet<String>,
    /// Files being read page by page: how many leading characters were seen.
    covered: BTreeMap<String, usize>,
    /// Files already merged by an earlier `merge` with `files`.
    merged: BTreeSet<String>,
}

impl Worker {
    /// A slot claimed before the worktree exists, so parallel picks see it as load.
    fn reserved(spec: &ModelSpec, repo: PathBuf, now_ms: u64) -> Self {
        Self {
            provider: spec.provider,
            model: spec.id.to_string(),
            repo,
            created_ms: now_ms,
            path: None,
            running: true,
            shown: BTreeSet::new(),
            covered: BTreeMap::new(),
            merged: BTreeSet::new(),
        }
    }

    /// Characters `from..to` of `file`'s diff (`total` long) were shown. The file counts as
    /// shown in full once its leading characters were seen without a gap.
    fn cover(&mut self, file: &str, from: usize, to: usize, total: usize) {
        if self.shown.contains(file) {
            return;
        }
        let seen = self.covered.get(file).copied().unwrap_or(0);
        if from > seen {
            return;
        }
        if seen.max(to) >= total {
            self.covered.remove(file);
            self.shown.insert(file.to_string());
        } else {
            self.covered.insert(file.to_string(), seen.max(to));
        }
    }
}

// --------------------------------------------------------------- what /workflows reads

/// The project folder as rows and records name it: canonical when it can be.
fn handoff_key(dir: &Path) -> String {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()).to_string_lossy().into_owned()
}

/// A worker that is still open (running, or done and waiting for the planner to merge or
/// discard it), as `/workflows` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWorker {
    pub worker_id: String,
    pub provider: String,
    pub model: String,
    pub created_ms: u64,
    pub running: bool,
    /// Files the planner has seen in full, and files already merged.
    pub shown: usize,
    pub merged: usize,
    /// Tool calls made so far, and the latest one (`Bash cargo test`).
    pub tools: usize,
    pub activity: Option<String>,
}

/// The open workers of the project `repo`, from the registry in `<state_dir>/workers`,
/// oldest first. Read only: nothing is adopted, changed or removed.
pub fn open_workers(state_dir: &Path, repo: &Path) -> Vec<OpenWorker> {
    let key = handoff_key(repo);
    let store = Store { dir: state_dir.join("workers") };
    let mut out: Vec<OpenWorker> = store
        .load()
        .into_iter()
        .filter(|s| handoff_key(&s.repo) == key)
        .map(|s| {
            let (tools, activity) = store.activity(&s.worker_id).map_or((0, None), |(n, a)| (n, Some(a)));
            OpenWorker { worker_id: s.worker_id, provider: s.provider, model: s.model, created_ms: s.created_ms, running: s.running, shown: s.shown.len(), merged: s.merged.len(), tools, activity }
        })
        .collect();
    out.sort_by_key(|w| w.created_ms);
    out
}

/// One worker attempt from the orchestration log.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Delegation {
    pub ts_ms: u64,
    pub provider: String,
    pub model: String,
    pub ok: bool,
    #[serde(default)]
    pub error_kind: Option<String>,
    pub wall_ms: u64,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub worker_id: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub attempt: Option<u32>,
    #[serde(default)]
    pub files_changed: Vec<String>,
    #[serde(default)]
    pub tool_calls: Vec<String>,
    #[serde(default)]
    pub repo: Option<String>,
}

/// The project's delegations from the log at `log`, newest last, at most `limit`. Rows
/// written before the log named the project are left out, as is anything unreadable.
pub fn recent_delegations(log: &Path, repo: &Path, limit: usize) -> Vec<Delegation> {
    let key = handoff_key(repo);
    let Ok(text) = std::fs::read_to_string(log) else { return Vec::new() };
    let mut rows: Vec<Delegation> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Delegation>(l).ok())
        .filter(|d| d.kind.as_deref() == Some("delegate") && d.repo.as_deref() == Some(key.as_str()))
        .collect();
    let skip = rows.len().saturating_sub(limit);
    rows.drain(..skip);
    rows
}

// --------------------------------------------------------------- who owns a record

/// A server process: its pid plus a start-time token that survives pid reuse (a new
/// process with a recycled pid has a different start time). `start` is `None` where the
/// platform cannot say, and the pid alone is compared then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Owner {
    pid: u32,
    #[serde(default)]
    start: Option<u64>,
}

/// Who is running. Real: [`ProcLiveness`]; tests inject a fake, so no test needs real pids.
trait Liveness: Send + Sync {
    /// This process.
    fn me(&self) -> Owner;
    /// Is that process still running? Unsure counts as yes: nothing of a live server may
    /// be touched, and a dead one's records only wait longer.
    fn alive(&self, owner: &Owner) -> bool;
}

/// The start time of `pid` in clock ticks since boot: field 22 of `/proc/<pid>/stat`,
/// counted after the `)` that ends the command name (which may itself hold spaces).
fn proc_start(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
}

/// Real liveness: `/proc` where there is one, else `kill -0` (pid only).
struct ProcLiveness;

impl Liveness for ProcLiveness {
    fn me(&self) -> Owner {
        let pid = std::process::id();
        Owner { pid, start: proc_start(pid) }
    }

    fn alive(&self, o: &Owner) -> bool {
        if Path::new("/proc/self").exists() {
            Path::new(&format!("/proc/{}", o.pid)).exists() && o.start.zip(proc_start(o.pid)).is_none_or(|(then, now)| then == now)
        } else {
            Command::new("kill").arg("-0").arg(o.pid.to_string()).stderr(Stdio::null()).status().map_or(true, |s| s.success())
        }
    }
}

/// A worker as written to disk. Plain data: provider as its name.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Saved {
    worker_id: String,
    path: PathBuf,
    repo: PathBuf,
    provider: String,
    model: String,
    created_ms: u64,
    #[serde(default)]
    running: bool,
    #[serde(default)]
    shown: BTreeSet<String>,
    #[serde(default)]
    covered: BTreeMap<String, usize>,
    #[serde(default)]
    merged: BTreeSet<String>,
    /// The server that owns the worker. A record without one (written before owners were
    /// stamped) counts as owned by a dead server.
    #[serde(default)]
    owner: Option<Owner>,
}

impl Saved {
    /// `None` for a slot with no worktree yet: there is nothing on disk to keep track of.
    fn of(id: &str, w: &Worker, owner: &Owner) -> Option<Self> {
        Some(Self {
            owner: Some(owner.clone()),
            worker_id: id.to_string(),
            path: w.path.clone()?,
            repo: w.repo.clone(),
            provider: w.provider.to_string(),
            model: w.model.clone(),
            created_ms: w.created_ms,
            running: w.running,
            shown: w.shown.clone(),
            covered: w.covered.clone(),
            merged: w.merged.clone(),
        })
    }

    fn into_worker(self) -> Option<(String, Worker)> {
        let provider = self.provider.parse().ok()?;
        let w = Worker {
            provider,
            model: self.model,
            repo: self.repo,
            created_ms: self.created_ms,
            path: Some(self.path),
            running: false,
            shown: self.shown,
            covered: self.covered,
            merged: self.merged,
        };
        Some((self.worker_id, w))
    }
}

/// Where worker records live. Every method blocks on disk: the [`Delegator`] calls them
/// only off the async threads and never under its registry lock. Real: [`Store`].
trait Records: Send + Sync {
    fn save(&self, s: &Saved);
    fn remove(&self, id: &str);
    fn load(&self) -> Vec<Saved>;
    /// The worker just began a tool call (`summary`), the `count`th. Kept beside its record
    /// so a screen can show what a running worker is doing; nothing depends on it.
    fn note_activity(&self, _id: &str, _count: usize, _summary: &str) {}
}

/// One JSON file per worker under `<state dir>/workers`, so two servers sharing a state
/// directory never overwrite each other's records. Writes are atomic (temp file, then
/// rename); a missing, unreadable or corrupt file is skipped, never an error.
struct Store {
    dir: PathBuf,
}

fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Store {
    fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn activity_file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.activity"))
    }

    /// How many tool calls the worker has made and the latest one, if it has made any.
    fn activity(&self, id: &str) -> Option<(usize, String)> {
        let text = std::fs::read_to_string(self.activity_file(id)).ok()?;
        let (count, summary) = text.split_once('\n')?;
        Some((count.parse().ok()?, summary.to_string()))
    }
}

impl Records for Store {
    fn save(&self, s: &Saved) {
        if !plain_id(&s.worker_id) || std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let tmp = self.dir.join(format!("{}.json.tmp", s.worker_id));
        let Ok(text) = serde_json::to_string(s) else { return };
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, self.file(&s.worker_id)).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn remove(&self, id: &str) {
        if plain_id(id) {
            let _ = std::fs::remove_file(self.file(id));
            let _ = std::fs::remove_file(self.activity_file(id));
        }
    }

    fn note_activity(&self, id: &str, count: usize, summary: &str) {
        if plain_id(id) && self.dir.is_dir() {
            let one_line: String = summary.split_whitespace().collect::<Vec<_>>().join(" ");
            let _ = std::fs::write(self.activity_file(id), format!("{count}\n{one_line}"));
        }
    }

    fn load(&self) -> Vec<Saved> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return vec![] };
        let mut out: Vec<Saved> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .filter_map(|p| serde_json::from_slice::<Saved>(&std::fs::read(p).ok()?).ok())
            .filter(|s| plain_id(&s.worker_id))
            .collect();
        out.sort_by(|a, b| a.worker_id.cmp(&b.worker_id));
        out
    }
}

/// True when `state` holds worker records or worktrees a previous run left, so a server
/// that never delegates can skip the cleanup.
pub fn has_leftovers(state: &Path) -> bool {
    let any = |d: PathBuf| std::fs::read_dir(d).is_ok_and(|mut e| e.next().is_some());
    any(state.join("workers")) || any(state.join("worktrees"))
}

/// A worker record as `/agents` shows it: plain facts, read from `<state>/workers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredWorker {
    pub id: String,
    pub provider: String,
    pub model: String,
    /// Still working (its server has not seen it finish).
    pub running: bool,
    pub created_ms: u64,
    /// Files the planner has already merged back.
    pub merged: Vec<String>,
}

/// The workers recorded under `state`, by id: the unmerged ones a planner can still
/// merge or discard.
pub fn stored_workers(state: &Path) -> Vec<StoredWorker> {
    Store { dir: state.join("workers") }
        .load()
        .into_iter()
        .map(|s| StoredWorker { id: s.worker_id, provider: s.provider, model: s.model, running: s.running, created_ms: s.created_ms, merged: s.merged.into_iter().collect() })
        .collect()
}

/// A worker whose record is older than this is abandoned: its checkout is removed.
const STALE_AFTER_MS: u64 = 24 * 60 * 60 * 1000;
/// A directory under the worktrees root that no record names is removed once it is this
/// old; younger ones may belong to a server that is creating them right now.
const ORPHAN_GRACE_MS: u64 = 10 * 60 * 1000;

/// A saved record and what recovery learned about it from the disk and the process table.
struct Found {
    saved: Saved,
    /// The checkout directory is still there.
    exists: bool,
    /// The owner is this process or another one still running.
    owner_alive: bool,
}

/// What recovery does with a record.
#[derive(Debug, PartialEq, Eq)]
enum Fate {
    /// Reload it, so merge and discard still work after a restart.
    Adopt,
    /// Remove its checkout and record.
    Drop,
    /// Leave it alone.
    Keep,
}

impl Found {
    /// A record whose owner is alive is never touched, however old: it is in use. Of a
    /// dead owner's: a finished worker is adopted; one that was still running when its
    /// server died is not reloaded (no planner was ever told its id) and waits to age out;
    /// and old or checkout-less ones are dropped.
    fn fate(&self, now_ms: u64) -> Fate {
        let s = &self.saved;
        if self.owner_alive {
            Fate::Keep
        } else if now_ms.saturating_sub(s.created_ms) > STALE_AFTER_MS || (!s.running && !self.exists) {
            Fate::Drop
        } else if s.running {
            Fate::Keep
        } else {
            Fate::Adopt
        }
    }
}

/// Everything recovery reads from disk, gathered in one blocking step.
struct Scan {
    records: Vec<Found>,
    /// Directories under the worktrees root, with their modification time.
    dirs: Vec<(PathBuf, Option<u64>)>,
}

impl Scan {
    fn gather(records: &dyn Records, liveness: &dyn Liveness, root: &Path) -> Self {
        let me = liveness.me();
        let records = records
            .load()
            .into_iter()
            .map(|saved| Found {
                exists: saved.path.exists(),
                owner_alive: saved.owner.as_ref().is_some_and(|o| *o == me || liveness.alive(o)),
                saved,
            })
            .collect();
        let dirs = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .map(|p| {
                let modified = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
                let ms = modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64);
                (p, ms)
            })
            .collect();
        Self { records, dirs }
    }
}

// --------------------------------------------------------------- the delegation API

/// What the planner asks for. There is deliberately no model field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DelegateRequest {
    pub brief: String,
    /// Hints about where the work is; not limits.
    pub files: Vec<String>,
    pub read_only: bool,
    /// The planner wants to review the diff itself even if the tests pass.
    pub review: bool,
}

/// Why a worker's changes were not merged on their own and wait for the planner's review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewReason {
    /// The planner asked to review.
    Asked,
    /// Auto-merge is switched off (`JEV_AUTO_MERGE=0`).
    Off,
    /// The project has no test command jev knows, so nothing proved the work.
    NoTestCommand,
    TestsFailed { command: String, output: String },
    TestsTimedOut { command: String, after: Duration },
    TestsNotRun { command: String, error: String },
    /// The tests passed but the patch does not apply cleanly to the user's tree.
    Conflict(String),
}

impl ReviewReason {
    fn tag(&self) -> &'static str {
        match self {
            Self::Asked => "review_asked",
            Self::Off => "review_off",
            Self::NoTestCommand => "review_no_tests",
            Self::TestsFailed { .. } => "review_tests_failed",
            Self::TestsTimedOut { .. } => "review_tests_timed_out",
            Self::TestsNotRun { .. } => "review_tests_not_run",
            Self::Conflict(_) => "review_conflict",
        }
    }
}

impl std::fmt::Display for ReviewReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Asked => f.write_str("you asked to review it"),
            Self::Off => f.write_str("auto-merge is off (JEV_AUTO_MERGE=0)"),
            Self::NoTestCommand => f.write_str("this project has no test command jev knows (Cargo, npm, Python or Go), so nothing proved the work"),
            Self::TestsFailed { command, output } => write!(f, "`{command}` failed in the worker's worktree. The end of its output:\n{output}"),
            Self::TestsTimedOut { command, after } => write!(f, "`{command}` ran longer than {} minutes in the worker's worktree and was stopped", after.as_secs() / 60),
            Self::TestsNotRun { command, error } => write!(f, "`{command}` could not be run: {error}"),
            Self::Conflict(why) => write!(f, "the tests passed, but the changes do not apply cleanly to your working tree: {why}"),
        }
    }
}

/// How a successful worker's work reached the planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landing {
    /// A read-only worker: a report and nothing to merge. `discarded` lists files it changed
    /// anyway in its throwaway worktree (empty when it ran in place or kept its word).
    Report { discarded: Vec<String> },
    /// It finished without changing any file.
    NoChanges,
    /// Its tests passed and its changes are in the user's working tree; `undo` reverses them.
    AutoMerged { command: String },
    /// Its worktree is kept for the planner to review, then merge or discard.
    Review(ReviewReason),
}

impl Landing {
    fn tag(&self) -> &'static str {
        match self {
            Self::Report { .. } => "report",
            Self::NoChanges => "no_changes",
            Self::AutoMerged { .. } => "auto_merged",
            Self::Review(r) => r.tag(),
        }
    }
}

/// What comes back to the planner.
#[derive(Debug, Clone)]
pub struct DelegateResult {
    pub worker_id: String,
    pub provider: Option<ProviderId>,
    pub model: Option<String>,
    /// The worker's final text, truncated.
    pub summary: String,
    /// EVERY file the worker changed, with its +/- counts, never cut short.
    pub stats: Vec<FileStat>,
    /// The diffs of the files in `shown`, whole files only, up to [`MAX_DIFF_CHARS`].
    pub diff: String,
    /// The files whose full diff is in `diff`. The rest need `show_diff` before a merge.
    pub shown: Vec<String>,
    pub ok: bool,
    pub error: Option<String>,
    /// What happened to earlier attempts (a limit hit, a refused model).
    pub retries: Vec<String>,
    /// Where the work went. Only meaningful when `ok`.
    pub landing: Landing,
}

impl DelegateResult {
    pub fn failed(error: String, retries: Vec<String>) -> Self {
        Self {
            worker_id: String::new(),
            provider: None,
            model: None,
            summary: String::new(),
            stats: vec![],
            diff: String::new(),
            shown: vec![],
            ok: false,
            error: Some(error),
            retries,
            landing: Landing::NoChanges,
        }
    }

    pub fn files_changed(&self) -> Vec<String> {
        self.stats.iter().map(|s| s.path.clone()).collect()
    }

    /// The text the planner reads.
    pub fn render(&self) -> String {
        let who = match (&self.provider, &self.model) {
            (Some(p), Some(m)) => format!("{p} {m}"),
            _ => "no worker".to_string(),
        };
        let status = match (self.ok, &self.landing) {
            (false, _) => "FAILED",
            (true, Landing::Report { .. }) => "done (read-only)",
            (true, Landing::AutoMerged { .. }) => "done, auto-merged",
            (true, Landing::NoChanges) => "done, no changes",
            (true, Landing::Review(_)) => "done, waiting for your review",
        };
        let mut out = vec![format!("worker {} ({who}): {status}", self.worker_id)];
        out.extend(self.retries.iter().map(|r| format!("retry: {r}")));
        if let Some(e) = &self.error {
            out.push(format!("error: {e}"));
        }
        if !self.ok {
            return out.join("\n");
        }
        let id = &self.worker_id;
        let reason = match &self.landing {
            Landing::Report { discarded } => {
                out.push(format!("worker report:\n{}", self.summary));
                if !discarded.is_empty() {
                    out.push(format!("It changed files although the piece was read-only; they were thrown away: {}.", discarded.join(", ")));
                }
                return out.join("\n");
            }
            Landing::NoChanges => {
                out.push("files changed: none".into());
                out.push(format!("worker report:\n{}", self.summary));
                return out.join("\n");
            }
            Landing::AutoMerged { command } => {
                self.push_files_and_report(&mut out);
                out.push(format!(
                    "Auto-merged into your working tree: `{command}` passed in the worker's worktree and the changes applied cleanly. \
                     undo({id}) takes exactly these changes back out."
                ));
                return out.join("\n");
            }
            Landing::Review(reason) => reason,
        };
        self.push_files_and_report(&mut out);
        out.push(format!("Not auto-merged: {reason}"));
        let shown = if self.shown.is_empty() { "no file".to_string() } else { self.shown.join(", ") };
        out.push(format!("diff (full diffs of: {shown}):\n{}", if self.diff.is_empty() { "(empty)" } else { &self.diff }));
        let unseen: Vec<&str> = self.stats.iter().map(|s| s.path.as_str()).filter(|p| !self.shown.iter().any(|s| s == p)).collect();
        if !unseen.is_empty() {
            out.push(format!(
                "NOT SHOWN here (too long): {}. Call show_diff({id}, file) for each to read it; merge refuses files you have not seen in full.",
                unseen.join(", ")
            ));
        }
        out.push(format!("Review the diff, then merge({id}) to apply it (or merge({id}, files) for some files) or discard({id}) to drop it."));
        out.join("\n")
    }

    fn push_files_and_report(&self, out: &mut Vec<String>) {
        out.push(format!("files changed ({}):", self.stats.len()));
        out.extend(self.stats.iter().map(|s| format!("  {}", s.render())));
        out.push(format!("worker report:\n{}", self.summary));
    }
}

/// What the MCP server calls. Implemented by [`Delegator`]; tests use a fake.
pub trait DelegateApi: Send + Sync {
    /// Start a delegation in the background; its task id, or why delegation is unavailable.
    fn start(&self, req: DelegateRequest) -> BoxFut<'_, Result<String, String>>;
    /// Collect background tasks (`task_ids`, or all), waiting at most `timeout` when given.
    fn wait(&self, task_ids: Option<Vec<String>>, timeout: Option<Duration>) -> BoxFut<'_, Result<String, String>>;
    /// One page of a worker's diff, a single file's when `file` is given, from character
    /// `offset`. What it shows in full counts as seen for `merge`.
    fn show_diff(&self, worker_id: String, file: Option<String>, offset: usize) -> BoxFut<'_, Result<String, String>>;
    /// Apply the worker's diff (only `files` when given) to the main tree, refusing any
    /// file not shown in full; remove its worktree once nothing is left.
    fn merge(&self, worker_id: String, files: Option<Vec<String>>) -> BoxFut<'_, Result<String, String>>;
    /// Remove the worker's worktree without applying anything.
    fn discard(&self, worker_id: String) -> BoxFut<'_, Result<String, String>>;
    /// Take an auto-merged worker's changes back out of the main tree.
    fn undo(&self, worker_id: String) -> BoxFut<'_, Result<String, String>>;
}

// --------------------------------------------------------------- the delegator

/// Jev's judgment of a brief: the same single batched request routing uses.
pub type Judge = Box<dyn Fn(String) -> BoxFut<'static, Option<ProviderWant>> + Send + Sync>;
type Clock = Box<dyn Fn() -> u64 + Send + Sync>;

/// A [`Judge`] that asks `router` (`Router::provider_want`, one HTTP request).
pub fn router_judge(router: Arc<Router>) -> Judge {
    Box::new(move |brief| {
        let router = router.clone();
        // A worker never orchestrates, so the sequence veto is not asked.
        Box::pin(async move { router.provider_want(&brief, "", None, false).await })
    })
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn unknown_worker(id: &str) -> String {
    format!("unknown worker id `{id}` (already merged or discarded?)")
}

/// A tag unique to one [`Delegator`]: the pid and the nanosecond clock tell servers apart
/// (ids are also record file names in a state directory servers share), the instance
/// count tells apart two made in one process at the same instant. Short and filename-safe.
fn run_tag(pid: u32, nanos: u128, instance: u64) -> String {
    format!("{pid:x}_{:x}_{instance:x}", nanos & 0xFF_FFFF_FFFF)
}

/// Where a worktree being created stands, shared between the blocking task making it and
/// the [`Reservation`] that may be dropped meanwhile.
#[derive(Default)]
struct CreateSlot {
    /// The delegation was dropped: whatever gets made must be removed again.
    cancelled: bool,
    /// The worktree, once made and not cancelled.
    path: Option<PathBuf>,
}

/// `create`, then record the path in `slot`, all-or-nothing against a cancel: whichever of
/// this and [`Reservation`]'s drop takes the slot lock second sees the other's work and
/// removes the worktree, so none is ever left behind.
fn create_unless_cancelled(w: &dyn Worktrees, id: &str, slot: &Mutex<CreateSlot>) -> Result<PathBuf> {
    let path = w.create(id)?;
    let mut s = lock(slot);
    if s.cancelled {
        drop(s);
        let _ = w.remove(&path);
        bail!("the delegation was cancelled");
    }
    s.path = Some(path.clone());
    Ok(path)
}

/// Owns a worker's reserved slot while its delegation runs. Dropped before [`disarm`] (the
/// `delegate` future was dropped, so nobody will ever merge or discard this worker) it
/// removes the placeholder from the registry, its record and its worktree, so a cancelled
/// call is not counted as load forever.
///
/// [`disarm`]: Reservation::disarm
struct Reservation<'a> {
    delegator: &'a Delegator,
    id: String,
    slot: Arc<Mutex<CreateSlot>>,
    armed: bool,
}

impl<'a> Reservation<'a> {
    fn new(delegator: &'a Delegator, id: &str) -> Self {
        Self { delegator, id: id.to_string(), slot: Arc::default(), armed: true }
    }

    fn slot(&self) -> Arc<Mutex<CreateSlot>> {
        self.slot.clone()
    }

    /// The delegation ended on its own: the registry already says what is left.
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let d = self.delegator;
        lock(&d.live).remove(&self.id);
        let path = {
            let mut s = lock(&self.slot);
            s.cancelled = true;
            s.path.take()
        };
        // Drop cannot await, and the disk work must not run on whichever thread dropped
        // the future: a plain thread, which takes its turn among the record writers.
        let (worktrees, records, gate, id) = (d.worktrees.clone(), d.records.clone(), d.gate.clone(), self.id.clone());
        std::thread::spawn(move || {
            let _turn = gate.blocking_lock();
            if let Some(path) = path {
                let _ = worktrees.remove(&path);
            }
            if let Some(records) = records {
                records.remove(&id);
            }
        });
    }
}

pub struct Delegator {
    runtime: Mutex<Runtime>,
    judge: Judge,
    worktrees: Arc<dyn Worktrees>,
    backend_factory: BackendFactory,
    /// The registry. Never held across IO or an await.
    live: Mutex<HashMap<String, Worker>>,
    records: Option<Arc<dyn Records>>,
    /// One record write at a time. Each takes a fresh snapshot of the registry when its turn
    /// comes, so the file always ends up as the registry is, whatever the order of writers.
    gate: Arc<tokio::sync::Mutex<()>>,
    liveness: Arc<dyn Liveness>,
    /// This server, stamped on every record it writes.
    owner: Owner,
    counter: AtomicU64,
    /// Makes worker ids unique across servers that share a state directory.
    run_tag: String,
    now: Clock,
    idle_timeout: Duration,
    log_path: Option<String>,
    planner: String,
    /// The privacy policy (and audit log) briefs answer to, as the planner's own turns do.
    guards: crate::turn::application::guards::Guards,
    /// Runs a finished worker's tests before its work may merge on its own.
    checks: Arc<dyn Checks>,
    /// Work whose tests pass merges without the planner's review (`JEV_AUTO_MERGE=0` turns it off).
    auto_merge: bool,
    /// One change to the main tree at a time: applies and undos share its index and the
    /// snapshot a failed apply restores, so two at once could undo each other's rollback.
    main_tree: tokio::sync::Mutex<()>,
    /// What each auto-merged worker applied, so `undo` can take exactly that back out.
    /// Kept for this server's life only.
    merged: Arc<Mutex<HashMap<String, Applied>>>,
    /// Delegations started in the background and not yet collected by `wait`, by number.
    tasks: Mutex<BTreeMap<u64, Task>>,
    next_task: AtomicU64,
}

/// A delegation running in the background ([`Delegator::start`]).
struct Task {
    started_ms: u64,
    /// Turns `true` once `outcome` is set. A sender dropped before that means the task died.
    done: tokio::sync::watch::Receiver<bool>,
    outcome: Arc<Mutex<Option<Finished>>>,
}

impl Task {
    fn ended(&self) -> bool {
        *self.done.borrow() || self.done.has_changed().is_err()
    }

    /// The outcome of an ended task; a task that died without one is a failure.
    fn into_finished(self) -> Finished {
        lock(&self.outcome).take().unwrap_or_else(|| Finished::done(DelegateResult::failed("the delegated task stopped unexpectedly".into(), vec![])))
    }
}

/// What `wait` found for one task it was asked about.
enum Collected {
    Ended(Task),
    Running { started_ms: u64 },
    /// Another `wait` collected it meanwhile.
    Gone,
}

fn task_id(n: u64) -> String {
    format!("t{n}")
}

fn task_number(id: &str) -> Option<u64> {
    id.strip_prefix('t')?.parse().ok()
}

/// An auto-merge, as `undo` needs it.
#[derive(Debug, Clone)]
struct Applied {
    patch: String,
    files: Vec<String>,
}

/// One worker attempt that failed, in the words the planner and the log need.
struct AttemptFailure {
    id: String,
    spec: &'static ModelSpec,
    err: TurnError,
}

impl Delegator {
    /// Real backends, clock and idle timeout; no log, unknown planner, workers not saved.
    pub fn new(runtime: Runtime, judge: Judge, worktrees: Box<dyn Worktrees>) -> Self {
        static INSTANCES: AtomicU64 = AtomicU64::new(0);
        let liveness: Arc<dyn Liveness> = Arc::new(ProcLiveness);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        Self {
            runtime: Mutex::new(runtime),
            judge,
            worktrees: Arc::from(worktrees),
            backend_factory: Box::new(crate::providers::application::provider_registry::backend_for),
            live: Mutex::new(HashMap::new()),
            records: None,
            gate: Arc::new(tokio::sync::Mutex::new(())),
            owner: liveness.me(),
            liveness,
            counter: AtomicU64::new(1),
            run_tag: run_tag(std::process::id(), nanos, INSTANCES.fetch_add(1, Ordering::SeqCst)),
            now: Box::new(log::now_ms),
            idle_timeout: idle_timeout_from_env(),
            log_path: None,
            planner: String::new(),
            guards: crate::turn::application::guards::Guards::default(),
            checks: Arc::new(ProjectChecks::default()),
            auto_merge: true,
            main_tree: tokio::sync::Mutex::new(()),
            merged: Arc::default(),
            tasks: Mutex::new(BTreeMap::new()),
            next_task: AtomicU64::new(1),
        }
    }

    /// How a finished worker's tests are found and run.
    pub fn with_checks(mut self, checks: Box<dyn Checks>) -> Self {
        self.checks = Arc::from(checks);
        self
    }

    /// `false` sends every editing worker's diff to the planner for review.
    pub fn with_auto_merge(mut self, on: bool) -> Self {
        self.auto_merge = on;
        self
    }

    /// The privacy policy, secret scan and audit log of the user's settings
    /// ([`crate::turn::application::guards::Guards::load`]). Without it briefs still get the default secret scan.
    pub fn with_guards(mut self, guards: crate::turn::application::guards::Guards) -> Self {
        self.guards = guards;
        self
    }

    pub fn with_backend_factory(mut self, f: BackendFactory) -> Self {
        self.backend_factory = f;
        self
    }

    pub fn with_clock(mut self, f: Clock) -> Self {
        self.now = f;
        self
    }

    pub fn with_idle_timeout(mut self, d: Duration) -> Self {
        self.idle_timeout = d;
        self
    }

    /// Append one row per worker attempt to this file (`orchestration.jsonl`).
    pub fn with_log(mut self, path: Option<String>) -> Self {
        self.log_path = path;
        self
    }

    /// The planner's `provider:model`, for the log.
    pub fn with_planner(mut self, planner: String) -> Self {
        self.planner = planner;
        self
    }

    /// Keep the worker registry in `<state_dir>/workers`, so a restarted server still
    /// knows its unmerged workers. Call [`Delegator::recover`] after.
    pub fn with_store(self, state_dir: &Path) -> Self {
        self.with_records(Arc::new(Store { dir: state_dir.join("workers") }))
    }

    fn with_records(mut self, records: Arc<dyn Records>) -> Self {
        self.records = Some(records);
        self
    }

    /// Who is running, for stamping records and for telling live servers from dead ones.
    #[cfg(test)]
    fn with_liveness(mut self, liveness: Arc<dyn Liveness>) -> Self {
        self.owner = liveness.me();
        self.liveness = liveness;
        self
    }

    fn next_id(&self) -> String {
        format!("w{}-{}", self.run_tag, self.counter.fetch_add(1, Ordering::SeqCst))
    }

    /// The ONE place blocking work leaves the async threads: `f` runs on the blocking pool,
    /// so slow disk and git work of parallel workers overlaps instead of queueing up.
    async fn off_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
        tokio::task::spawn_blocking(f).await.map_err(|e| anyhow!("blocking task failed: {e}"))
    }

    /// Git work, through [`Worktrees`], via [`Delegator::off_thread`].
    async fn blocking<T: Send + 'static>(&self, f: impl FnOnce(&dyn Worktrees) -> Result<T> + Send + 'static) -> Result<T> {
        let worktrees = self.worktrees.clone();
        Self::off_thread(move || f(&*worktrees)).await?
    }

    // ---- the registry

    /// Write the worker's record as the registry now has it: saved when it has a worktree,
    /// removed when it is gone, nothing while it is only a reserved slot. The registry
    /// lock is held just to copy the entry; the IO runs on the blocking pool, in turn.
    async fn persist(&self, id: &str) {
        let Some(records) = self.records.clone() else { return };
        let _turn = self.gate.lock().await;
        let id = id.to_string();
        let snapshot = lock(&self.live).get(&id).map(|w| Saved::of(&id, w, &self.owner));
        let _ = Self::off_thread(move || match snapshot {
            Some(Some(saved)) => records.save(&saved),
            Some(None) => {}
            None => records.remove(&id),
        })
        .await;
    }

    /// Change a worker's entry and save it. `None` when the id is unknown.
    async fn update<R: Send>(&self, id: &str, f: impl FnOnce(&mut Worker) -> R) -> Option<R> {
        let r = f(lock(&self.live).get_mut(id)?);
        self.persist(id).await;
        Some(r)
    }

    fn lookup(&self, id: &str) -> Result<Worker, String> {
        lock(&self.live).get(id).cloned().ok_or_else(|| unknown_worker(id))
    }

    /// Forget a worker (and its saved record), returning its entry.
    async fn take(&self, id: &str) -> Option<Worker> {
        let w = lock(&self.live).remove(id)?;
        self.persist(id).await;
        Some(w)
    }

    /// Pick a worker for a brief AND claim its slot under one lock, so parallel delegations
    /// see each other and fan out. Lock order is runtime, then the registry.
    fn reserve(&self, signals: &BriefSignals, excluded: &HashSet<(ProviderId, String)>, brief_len: usize, id: &str) -> Result<&'static ModelSpec, NoWorker> {
        let runtime = lock(&self.runtime);
        let mut live = lock(&self.live);
        let spec = {
            let usable = runtime.usable((self.now)()).without(excluded);
            let running = |p: ProviderId| live.values().filter(|w| w.running && w.provider == p).count();
            let utilization = |p: ProviderId| runtime.plan_utilization(p);
            pick_worker(
                signals,
                &WorkerView {
                    usable: &usable,
                    allow: &runtime.allow,
                    overrides: &runtime.overrides,
                    plan_utilization: &utilization,
                    live: &running,
                    context_tokens: (brief_len / 4) as u64,
                },
            )?
        };
        live.insert(id.to_string(), Worker::reserved(spec, self.worktrees.repo(), (self.now)()));
        Ok(spec)
    }

    /// Remove a worker's worktree and forget it. A failed removal is reported but the
    /// worker is forgotten anyway: a worktree that cannot be removed is a leftover to
    /// clean by hand (`git worktree prune`), not a reason to keep tracking it.
    async fn forget(&self, id: &str) -> Result<(), String> {
        let worker = self.take(id).await.ok_or_else(|| unknown_worker(id))?;
        let Some(path) = worker.path else { return Ok(()) };
        let shown = path.clone();
        self.blocking(move |w| w.remove(&path)).await.map_err(|e| format!("worker {id} forgotten, but its worktree {} was not removed: {e}", shown.display()))
    }

    /// At startup: reload the workers an earlier run left, and clean up. Records of a
    /// server that is still running are NEVER touched, adopted or removed: they are in use.
    /// Of a dead server's, finished workers are reloaded (stamped as this server's now),
    /// and those older than 24 hours lose their checkout and record. Directories under the
    /// worktrees root that no record names (and that are not brand new) go too; `git
    /// worktree prune` runs in every repository touched. All disk work is off the async threads.
    pub async fn recover(&self) {
        let Some(records) = self.records.clone() else { return };
        let (liveness, root) = (self.liveness.clone(), self.worktrees.root());
        let Ok(scan) = Self::off_thread(move || Scan::gather(&*records, &*liveness, &root)).await else { return };
        let now = (self.now)();
        let known: HashSet<&PathBuf> = scan.records.iter().map(|f| &f.saved.path).collect();
        let mut repos: BTreeSet<PathBuf> = BTreeSet::new();
        for found in &scan.records {
            match found.fate(now) {
                Fate::Keep => {}
                Fate::Drop => {
                    self.drop_checkout(found.saved.path.clone()).await;
                    self.persist(&found.saved.worker_id).await;
                    repos.insert(found.saved.repo.clone());
                }
                Fate::Adopt => {
                    if let Some((id, w)) = found.saved.clone().into_worker() {
                        lock(&self.live).insert(id.clone(), w);
                        self.persist(&id).await;
                    }
                }
            }
        }
        for (path, modified_ms) in scan.dirs.iter().filter(|(p, _)| !known.contains(p)) {
            if now.saturating_sub(modified_ms.unwrap_or(now)) > ORPHAN_GRACE_MS {
                self.drop_checkout(path.clone()).await;
                repos.insert(self.worktrees.repo());
            }
        }
        for repo in repos {
            let _ = self.blocking(move |w| w.prune(&repo)).await;
        }
    }

    async fn drop_checkout(&self, path: PathBuf) {
        let _ = self.blocking(move |w| w.remove(&path)).await;
    }

    // ---- delegating

    /// Run the worker CLI in its worktree. Plan usage it reports feeds the spreading.
    async fn run_worker(&self, id: &str, spec: &'static ModelSpec, dir: &Path, prompt: String, mode: Mode) -> (Result<TurnOutcome, TurnError>, Vec<String>) {
        let backend = (self.backend_factory)(spec.provider);
        let req = TurnRequest { dir: dir.to_path_buf(), model: spec.id.to_string(), prompt, images: vec![], session_id: None, mode, trust_project_settings: crate::approval::application::extension_hooks::project_settings_trusted(dir) };
        let mut tools: Vec<String> = Vec::new();
        let mut on_event = |e: &Event| match e {
            Event::Tool(summary) => {
                tools.push(summary.clone());
                if let Some(records) = &self.records {
                    records.note_activity(id, tools.len(), summary);
                }
            }
            Event::Plan(plan) => lock(&self.runtime).note_plan(spec.provider, plan.clone()),
            _ => {}
        };
        let result = backend::run_turn(&*backend, &req, self.idle_timeout, &mut on_event).await;
        (result, tools)
    }

    fn log_attempt(&self, a: &AttemptLog) {
        let Some(path) = &self.log_path else { return };
        let reason = format!("delegated {} brief", a.task);
        let mut row = turn_row(a.spec.provider, a.spec.id, None, &reason, a.jev, a.result, a.wall_ms);
        if let Value::Object(o) = &mut row {
            o.insert("kind".into(), json!("delegate"));
            o.insert("planner".into(), json!(self.planner));
            o.insert("worker_id".into(), json!(a.id));
            o.insert("task".into(), json!(a.task.as_str()));
            o.insert("attempt".into(), json!(a.attempt));
            // When the worker ran, so parallel workers' overlap can be read off the log. The finish
            // is the start plus the measured wall time, so it can never precede the start even if
            // the wall clock steps back mid-run.
            o.insert("started_at".into(), json!(a.started_ms));
            o.insert("finished_at".into(), json!(a.started_ms.saturating_add(a.wall_ms)));
            o.insert("duration_ms".into(), json!(a.wall_ms));
            o.insert("read_only".into(), json!(a.read_only));
            o.insert("files_changed".into(), json!(a.files));
            o.insert("landing".into(), json!(a.landing));
            o.insert("in_place".into(), json!(a.in_place));
            o.insert("tool_calls".into(), json!(a.tools));
            o.insert("repo".into(), json!(handoff_key(&self.worktrees.repo())));
        }
        log::append(path, &row);
    }

    /// One attempt: pick and claim, create the worktree, run. `Err` is the failure to react
    /// to. If the future is dropped on the way (the planner cancelled the call) the
    /// [`Reservation`] removes the placeholder, its record and any worktree already made.
    async fn attempt(
        &self,
        req: &DelegateRequest,
        signals: &BriefSignals,
        jev: &Value,
        excluded: &HashSet<(ProviderId, String)>,
        attempt: u32,
    ) -> Result<Finished, AttemptOutcome> {
        let id = self.next_id();
        let spec = self.reserve(signals, excluded, req.brief.len(), &id).map_err(|e| AttemptOutcome::Stop(e.to_string()))?;
        let reservation = Reservation::new(self, &id);
        let out = self.run_attempt(req, signals, jev, spec, id, &reservation, attempt).await;
        reservation.disarm();
        out
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_attempt(
        &self,
        req: &DelegateRequest,
        signals: &BriefSignals,
        jev: &Value,
        spec: &'static ModelSpec,
        id: String,
        reservation: &Reservation<'_>,
        attempt: u32,
    ) -> Result<Finished, AttemptOutcome> {
        // A read-only worker on a CLI that enforces plan mode reads the user's own folder (and
        // so sees uncommitted edits); every other worker gets a worktree.
        let in_place = req.read_only && plan_mode_enforced(spec.provider);
        let path = if in_place {
            self.worktrees.repo()
        } else {
            let made = self.blocking({
                let (id, slot) = (id.clone(), reservation.slot());
                move |w| create_unless_cancelled(w, &id, &slot)
            });
            match made.await {
                Ok(path) => path,
                Err(e) => {
                    self.take(&id).await;
                    return Err(AttemptOutcome::Stop(format!("could not create a worktree: {e:#}")));
                }
            }
        };
        if !in_place {
            self.update(&id, |w| w.path = Some(path.clone())).await;
        }
        let started = Instant::now();
        let started_ms = (self.now)();
        // Secrets are blanked as for any provider, and the audit log notes what was sent.
        let prompt = self.guards.outgoing(&worker_prompt(&req.brief, &req.files, req.read_only), spec.provider, spec.id, (self.now)()).prompt;
        let mode = if req.read_only { Mode::Plan } else { Mode::default() };
        let (result, tools) = self.run_worker(&id, spec, &path, prompt, mode).await;
        self.update(&id, |w| w.running = false).await;
        let wall_ms = started.elapsed().as_millis() as u64;
        let task = worker_task(signals);
        let log = |result: &Result<TurnOutcome, TurnError>, files: &[String], landing: Option<&'static str>| {
            self.log_attempt(&AttemptLog { spec, id: &id, task, attempt, read_only: req.read_only, jev, result, wall_ms, started_ms, files, tools: &tools, landing, in_place })
        };
        let out = match result {
            Err(err) => {
                log(&Err(err.clone()), &[], None);
                // A failed worker's partial work is never offered; its worktree goes.
                if in_place {
                    self.take(&id).await;
                } else {
                    let _ = self.forget(&id).await;
                }
                return Err(AttemptOutcome::Failed(AttemptFailure { id, spec, err }));
            }
            Ok(out) => out,
        };
        let summary = truncate_with_note(out.text.trim(), MAX_SUMMARY_CHARS, "report");
        let done = |landing: Landing, stats: Vec<FileStat>, diff: String, shown: Vec<String>| DelegateResult {
            worker_id: id.clone(),
            provider: Some(spec.provider),
            model: Some(spec.id.to_string()),
            summary: summary.clone(),
            stats,
            diff,
            shown,
            ok: true,
            error: None,
            retries: vec![],
            landing,
        };
        if in_place {
            self.take(&id).await;
            let landing = Landing::Report { discarded: vec![] };
            log(&Ok(out), &[], Some(landing.tag()));
            return Ok(Finished::done(done(landing, vec![], String::new(), vec![])));
        }
        let read = self.blocking({
            let path = path.clone();
            move |w| w.diff(&path)
        });
        let diff = match read.await {
            Ok(diff) => diff,
            Err(e) => {
                log(&Ok(out), &[], None);
                let _ = self.forget(&id).await;
                return Err(AttemptOutcome::Stop(format!("worker {id} finished but its changes could not be read: {e:#}")));
            }
        };
        let parts = split_patch(&diff);
        let stats: Vec<FileStat> = parts.iter().map(FileDiff::stat).collect();
        let files: Vec<String> = stats.iter().map(|s| s.path.clone()).collect();
        // A read-only worker's worktree, and an editor's that changed nothing, only held a report.
        if req.read_only || parts.is_empty() {
            let _ = self.forget(&id).await;
            let landing = if req.read_only { Landing::Report { discarded: files.clone() } } else { Landing::NoChanges };
            log(&Ok(out), &files, Some(landing.tag()));
            return Ok(Finished::done(done(landing, vec![], String::new(), vec![])));
        }
        let verdict = self.verify(&path, req.review, reservation.slot()).await;
        log(&Ok(out), &files, Some(verdict.tag()));
        // The landing is decided by `settle`, when nobody is editing the main tree.
        Ok(Finished::Pending(Box::new(Pending { result: done(Landing::NoChanges, stats, String::new(), vec![]), patch: diff, verdict })))
    }

    /// Whether a finished editing worker's work may merge on its own: the project's tests,
    /// run in its worktree, must pass. Touches nothing outside the worktree.
    async fn verify(&self, worktree: &Path, review: bool, slot: Arc<Mutex<CreateSlot>>) -> Verdict {
        if review {
            return Verdict::Review(ReviewReason::Asked);
        }
        if !self.auto_merge {
            return Verdict::Review(ReviewReason::Off);
        }
        let (checks, dir) = (self.checks.clone(), worktree.to_path_buf());
        // The run stops early if the delegation is cancelled: its worktree is going away.
        let ran = Self::off_thread(move || {
            let command = checks.test_command(&dir)?;
            let run = checks.run(&dir, &command, &|| lock(&slot).cancelled);
            Some((command, run))
        });
        let (command, run) = match ran.await {
            Ok(Some(found)) => found,
            Ok(None) => return Verdict::Review(ReviewReason::NoTestCommand),
            Err(e) => return Verdict::Review(ReviewReason::TestsNotRun { command: "the tests".into(), error: format!("{e:#}") }),
        };
        Verdict::Review(match run {
            CheckRun::Passed => return Verdict::Passed { command },
            CheckRun::Failed { output } => ReviewReason::TestsFailed { command, output },
            CheckRun::TimedOut { after } => ReviewReason::TestsTimedOut { command, after },
            CheckRun::Stopped => ReviewReason::TestsNotRun { command, error: "the delegation was cancelled".into() },
            CheckRun::CouldNotStart { error } => ReviewReason::TestsNotRun { command, error },
        })
    }

    /// Land a finished delegation in the main tree: a worker whose tests passed is applied
    /// (or goes to review when its patch conflicts); one under review gets its first diff
    /// page. Called when the planner is not editing: inside `delegate` or `wait`.
    async fn settle(&self, finished: Finished) -> DelegateResult {
        let pending = match finished {
            Finished::Done(result) => return *result,
            Finished::Pending(pending) => *pending,
        };
        let Pending { mut result, patch, verdict } = pending;
        let reason = match verdict {
            Verdict::Review(reason) => reason,
            Verdict::Passed { command } => match self.auto_apply(&result.worker_id, &patch, result.files_changed()).await {
                Ok(()) => {
                    // A worktree that will not go is reported by `forget` and cleaned by recovery later.
                    let _ = self.forget(&result.worker_id).await;
                    result.landing = Landing::AutoMerged { command };
                    return result;
                }
                Err(why) => ReviewReason::Conflict(why),
            },
        };
        let (diff, shown) = first_page(&split_patch(&patch), MAX_DIFF_CHARS);
        self.update(&result.worker_id, |w| w.shown.extend(shown.iter().cloned())).await;
        result.diff = diff;
        result.shown = shown;
        result.landing = Landing::Review(reason);
        result
    }

    /// Apply a tested worker's patch to the main tree, one change at a time. Recorded for
    /// `undo` in the same blocking step as the apply, so a call cancelled meanwhile still
    /// leaves its changes undoable.
    async fn auto_apply(&self, id: &str, patch: &str, files: Vec<String>) -> Result<(), String> {
        let _turn = self.main_tree.lock().await;
        let (merged, id, applied) = (self.merged.clone(), id.to_string(), Applied { patch: patch.to_string(), files });
        self.blocking(move |w| {
            w.apply(&applied.patch)?;
            lock(&merged).insert(id, applied);
            Ok(())
        })
        .await
        .map_err(|e| format!("{e:#}"))
    }

    /// Run a delegation to the end and land it: what a blocking call returns.
    pub async fn delegate(&self, req: DelegateRequest) -> DelegateResult {
        let finished = self.run(req).await;
        self.settle(finished).await
    }

    /// Start a delegation in the background and return its task id at once, so the planner
    /// keeps working meanwhile. The worker runs and is tested; nothing lands in the main tree
    /// until [`Delegator::wait`] collects it.
    pub fn start(self: &Arc<Self>, req: DelegateRequest) -> String {
        let n = self.next_task.fetch_add(1, Ordering::SeqCst);
        let (tx, done) = tokio::sync::watch::channel(false);
        let outcome = Arc::new(Mutex::new(None));
        lock(&self.tasks).insert(n, Task { started_ms: (self.now)(), done, outcome: outcome.clone() });
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let finished = me.run(req).await;
            *lock(&outcome) = Some(finished);
            let _ = tx.send(true);
        });
        task_id(n)
    }

    /// Wait for background tasks (`ids`, or every one not yet collected) to end, at most
    /// `timeout` when given, then land and report each that ended. Each task is collected
    /// once; the ones still running are listed.
    pub async fn wait(&self, ids: Option<Vec<String>>, timeout: Option<Duration>) -> Result<String, String> {
        let wanted = self.wanted_tasks(ids)?;
        if wanted.is_empty() {
            return Ok("no delegated tasks to wait for: every task has been collected".into());
        }
        let receivers: Vec<_> = {
            let tasks = lock(&self.tasks);
            wanted.iter().filter_map(|n| tasks.get(n)).map(|t| t.done.clone()).collect()
        };
        let deadline = timeout.map(|t| tokio::time::Instant::now() + t);
        for mut rx in receivers {
            // An error means the task died: it counts as ended and is reported below.
            let ended = async move {
                let _ = rx.wait_for(|done| *done).await;
            };
            let in_time = match deadline {
                Some(at) => tokio::time::timeout_at(at, ended).await.is_ok(),
                None => {
                    ended.await;
                    true
                }
            };
            if !in_time {
                break;
            }
        }
        let mut blocks: Vec<String> = Vec::new();
        let mut running: Vec<String> = Vec::new();
        for n in wanted {
            let collected = {
                let mut tasks = lock(&self.tasks);
                match tasks.get(&n) {
                    None => Collected::Gone,
                    Some(t) if t.ended() => tasks.remove(&n).map_or(Collected::Gone, Collected::Ended),
                    Some(t) => Collected::Running { started_ms: t.started_ms },
                }
            };
            match collected {
                Collected::Ended(task) => {
                    let result = self.settle(task.into_finished()).await;
                    blocks.push(format!("task {}: {}", task_id(n), result.render()));
                }
                Collected::Running { started_ms } => {
                    let took = crate::shared::domain::wording::human_duration((self.now)().saturating_sub(started_ms));
                    running.push(format!("{} (running for {took})", task_id(n)));
                }
                Collected::Gone => blocks.push(format!("task {}: already collected by another wait", task_id(n))),
            }
        }
        if !running.is_empty() {
            let waited = timeout.map_or(String::new(), |t| format!(" after {}", crate::shared::domain::wording::human_duration(t.as_millis() as u64)));
            blocks.push(format!("Still running{waited}: {}. Call wait again to collect them.", running.join(", ")));
        }
        Ok(blocks.join("\n\n"))
    }

    /// The task numbers `ids` names, each once, or every uncollected task when `None`.
    fn wanted_tasks(&self, ids: Option<Vec<String>>) -> Result<Vec<u64>, String> {
        let tasks = lock(&self.tasks);
        let Some(ids) = ids else { return Ok(tasks.keys().copied().collect()) };
        if ids.is_empty() {
            return Err("`task_ids` is empty: name the tasks, or leave it out to wait for every task".into());
        }
        let mut wanted: Vec<u64> = Vec::new();
        for id in &ids {
            let n = task_number(id).filter(|n| tasks.contains_key(n)).ok_or_else(|| {
                let open: Vec<String> = tasks.keys().map(|n| task_id(*n)).collect();
                let open = if open.is_empty() { "none".to_string() } else { open.join(", ") };
                format!("unknown task `{id}` (already collected?); uncollected tasks: {open}")
            })?;
            if !wanted.contains(&n) {
                wanted.push(n);
            }
        }
        Ok(wanted)
    }

    /// Judge the brief, then run workers until one succeeds or the attempts are used up.
    /// Nothing reaches the main tree here: that is [`Delegator::settle`]'s.
    async fn run(&self, req: DelegateRequest) -> Finished {
        // The brief is a prompt like any other: the privacy policy decides which providers may
        // have it, and Jev (a cloud service) is asked only when some cloud provider may.
        let repo = self.worktrees.repo();
        let outgoing = worker_prompt(&req.brief, &req.files, req.read_only);
        let verdict = self.guards.privacy.check(&repo, &outgoing, &[]);
        let mut excluded: HashSet<(ProviderId, String)> = HashSet::new();
        for p in crate::shared::ALL_PROVIDERS.into_iter().filter(|p| !verdict.permits(*p)) {
            excluded.extend(crate::catalog::domain::model_table::models().iter().filter(|m| m.provider == p).map(|m| (m.provider, m.id.to_string())));
        }
        if crate::shared::ALL_PROVIDERS.into_iter().all(|p| !verdict.permits(p)) {
            return Finished::done(DelegateResult::failed(format!("not delegated: {}", verdict.refusal_line()), Vec::new()));
        }
        let want = if self.guards.may_leave(&repo, &outgoing, &[]) {
            (self.judge)(crate::turn::domain::privacy::prepare(&self.guards.privacy, &req.brief).prompt).await
        } else {
            None
        };
        let signals = want.as_ref().map_or_else(BriefSignals::unjudged, |w| BriefSignals::from_want(w, req.read_only));
        let jev = want.map_or(Value::Null, |w| w.signals);
        let mut retries: Vec<String> = Vec::new();
        for attempt in 1..=MAX_WORKER_ATTEMPTS {
            match self.attempt(&req, &signals, &jev, &excluded, attempt).await {
                Ok(finished) => return finished.with_retries(retries),
                Err(AttemptOutcome::Stop(message)) => return Finished::done(DelegateResult::failed(message, retries)),
                Err(AttemptOutcome::Failed(f)) => {
                    let react = lock(&self.runtime).react(f.spec.provider, f.spec.id, &f.err, &mut excluded, (self.now)());
                    match react {
                        Some(note) if attempt < MAX_WORKER_ATTEMPTS => retries.push(note),
                        _ => {
                            let mut failed = DelegateResult::failed(format!("{} {} failed: {}", f.spec.provider, f.spec.id, f.err.message), retries);
                            failed.worker_id = f.id;
                            failed.provider = Some(f.spec.provider);
                            failed.model = Some(f.spec.id.to_string());
                            return Finished::done(failed);
                        }
                    }
                }
            }
        }
        Finished::done(DelegateResult::failed("no worker succeeded".into(), retries))
    }

    // ---- reviewing and merging

    /// A finished worker, and its diff split per file (files already merged left out).
    async fn finished_diff(&self, id: &str) -> Result<(Worker, Vec<FileDiff>), String> {
        let worker = self.lookup(id)?;
        let path = match (&worker.path, worker.running) {
            (Some(path), false) => path.clone(),
            _ => return Err(format!("worker {id} is still running")),
        };
        let diff = self.blocking(move |w| w.diff(&path)).await.map_err(|e| format!("could not read worker {id}'s changes: {e:#}"))?;
        let parts = split_patch(&diff).into_iter().filter(|p| !worker.merged.contains(&p.path)).collect();
        Ok((worker, parts))
    }

    pub async fn show_diff(&self, id: &str, file: Option<&str>, offset: usize) -> Result<String, String> {
        let (_, parts) = self.finished_diff(id).await?;
        if parts.is_empty() {
            return Ok(format!("worker {id} has no changes left to show"));
        }
        let all: Vec<String> = parts.iter().map(|p| p.path.clone()).collect();
        // The text to page, and where each file sits in it: (path, start, length) in characters.
        let (text, spans): (String, Vec<(String, usize, usize)>) = match file {
            Some(f) => {
                let part = parts.iter().find(|p| p.path == f).ok_or_else(|| format!("worker {id} changed no file `{f}`; files: {}", all.join(", ")))?;
                (part.text.clone(), vec![(part.path.clone(), 0, part.chars())])
            }
            None => {
                let mut at = 0;
                let spans = parts
                    .iter()
                    .map(|p| {
                        let start = at;
                        at += p.chars();
                        (p.path.clone(), start, p.chars())
                    })
                    .collect();
                (join_diffs(&parts), spans)
            }
        };
        let total = text.chars().count();
        if offset >= total {
            return Err(format!("offset {offset} is past the end ({total} characters)"));
        }
        let (page, end) = char_page(&text, offset, MAX_DIFF_CHARS);
        self.update(id, |w| {
            for (path, start, len) in &spans {
                let (lo, hi) = (offset.max(*start), end.min(start + len));
                if lo < hi {
                    w.cover(path, lo - start, hi - start, *len);
                }
            }
        })
        .await;
        let seen = self.lookup(id)?.shown;
        let unseen: Vec<&str> = all.iter().map(String::as_str).filter(|p| !seen.contains(*p)).collect();
        let what = file.map_or("all files".to_string(), |f| format!("file {f}"));
        let more = if end < total {
            let scope = file.map(|f| format!(", file \"{f}\"")).unwrap_or_default();
            format!("[more: call show_diff with worker_id \"{id}\"{scope}, offset {end}]")
        } else {
            "[end]".to_string()
        };
        let seen_line = if unseen.is_empty() { "Every file has now been shown in full.".to_string() } else { format!("Not yet shown in full: {}.", unseen.join(", ")) };
        Ok(format!("worker {id}, {what}, characters {offset}-{end} of {total}:\n{page}\n{more}\n{seen_line}"))
    }

    pub async fn merge(&self, id: &str, only: Option<Vec<String>>) -> Result<String, String> {
        let (worker, parts) = self.finished_diff(id).await?;
        if worker.repo != self.worktrees.repo() {
            return Err(format!(
                "worker {id} was made from {}, but this server works in {}: start the planner there to merge it, or discard({id})",
                worker.repo.display(),
                self.worktrees.repo().display()
            ));
        }
        if parts.is_empty() {
            self.forget(id).await?;
            return Ok(format!("worker {id} has nothing left to merge; its worktree is removed"));
        }
        let chosen: Vec<&FileDiff> = match &only {
            None => parts.iter().collect(),
            Some(names) if names.is_empty() => return Err("`files` is empty: name the files to merge, or leave it out to merge all".into()),
            Some(names) => {
                let mut picked: Vec<&FileDiff> = Vec::new();
                for name in names {
                    let part = parts.iter().find(|p| &p.path == name).ok_or_else(|| {
                        format!("worker {id} has no file `{name}` to merge; files: {}", parts.iter().map(|p| p.path.as_str()).collect::<Vec<_>>().join(", "))
                    })?;
                    if !picked.contains(&part) {
                        picked.push(part);
                    }
                }
                picked
            }
        };
        let unseen: Vec<&str> = chosen.iter().map(|p| p.path.as_str()).filter(|p| !worker.shown.contains(*p)).collect();
        if !unseen.is_empty() {
            let seen: Vec<&str> = chosen.iter().map(|p| p.path.as_str()).filter(|p| worker.shown.contains(*p)).collect();
            let alternative = if seen.is_empty() {
                "none of the requested files has been shown yet".to_string()
            } else {
                format!("or merge again with `files` = [{}], the ones you have seen", seen.join(", "))
            };
            return Err(format!(
                "refusing to merge {} file(s) you have not seen in full: {}. Call show_diff for each of them first ({alternative}). Nothing was changed.",
                unseen.len(),
                unseen.join(", ")
            ));
        }
        let patch = join_diffs(chosen.iter().copied());
        let done: Vec<String> = chosen.iter().map(|p| p.path.clone()).collect();
        let turn = self.main_tree.lock().await;
        let applied = self.blocking(move |w| w.apply(&patch)).await;
        drop(turn);
        applied.map_err(|e| format!("{e:#}\nWorker {id}'s worktree is kept; discard({id}) it or merge by hand."))?;
        self.update(id, |w| w.merged.extend(done.iter().cloned())).await;
        let left: Vec<&str> = parts.iter().map(|p| p.path.as_str()).filter(|p| !done.iter().any(|d| d == p)).collect();
        if !left.is_empty() {
            return Ok(format!(
                "merged from worker {id}: {}. Changes are in your working tree. Not merged yet, still in the worker's worktree: {}. Merge or discard({id}) them later.",
                done.join(", "),
                left.join(", ")
            ));
        }
        let note = self.forget(id).await.err().map(|e| format!(" ({e})")).unwrap_or_default();
        Ok(format!("merged worker {id}: {}. Changes are in your working tree.{note}", done.join(", ")))
    }

    pub async fn discard(&self, id: &str) -> Result<String, String> {
        self.forget(id).await.map(|()| format!("discarded worker {id}"))
    }

    /// Take an auto-merged worker's changes back out of the main tree, if those files have
    /// not changed since in a way that stops the reverse patch.
    pub async fn undo(&self, id: &str) -> Result<String, String> {
        let applied = lock(&self.merged).get(id).cloned().ok_or_else(|| {
            format!("no auto-merged worker `{id}` to undo (undo works for workers auto-merged since this delegate server started)")
        })?;
        let _turn = self.main_tree.lock().await;
        let patch = applied.patch.clone();
        self.blocking(move |w| w.unapply(&patch)).await.map_err(|e| format!("could not undo worker {id}: {e:#}"))?;
        lock(&self.merged).remove(id);
        Ok(format!("undid worker {id}: {} are back to how they were before it merged", applied.files.join(", ")))
    }
}

impl DelegateApi for Arc<Delegator> {
    fn start(&self, req: DelegateRequest) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Ok(Delegator::start(self, req)) })
    }

    fn wait(&self, task_ids: Option<Vec<String>>, timeout: Option<Duration>) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Delegator::wait(self, task_ids, timeout).await })
    }

    fn show_diff(&self, worker_id: String, file: Option<String>, offset: usize) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Delegator::show_diff(self, &worker_id, file.as_deref(), offset).await })
    }

    fn merge(&self, worker_id: String, files: Option<Vec<String>>) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Delegator::merge(self, &worker_id, files).await })
    }

    fn discard(&self, worker_id: String) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Delegator::discard(self, &worker_id).await })
    }

    fn undo(&self, worker_id: String) -> BoxFut<'_, Result<String, String>> {
        Box::pin(async move { Delegator::undo(self, &worker_id).await })
    }
}

/// What the tests said about an editing worker's work, before it lands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// The tests passed: it may merge on its own if it applies cleanly.
    Passed { command: String },
    Review(ReviewReason),
}

impl Verdict {
    fn tag(&self) -> &'static str {
        match self {
            Self::Passed { .. } => "tests_passed",
            Self::Review(r) => r.tag(),
        }
    }
}

/// A delegation that ran to its end. Nothing of it has touched the main tree yet.
enum Finished {
    /// Nothing to land: a failure, a read-only report, or no changes.
    Done(Box<DelegateResult>),
    /// An editing worker's changes, waiting for [`Delegator::settle`].
    Pending(Box<Pending>),
}

impl Finished {
    fn done(result: DelegateResult) -> Self {
        Self::Done(Box::new(result))
    }

    fn with_retries(mut self, retries: Vec<String>) -> Self {
        match &mut self {
            Self::Done(result) => result.retries = retries,
            Self::Pending(pending) => pending.result.retries = retries,
        }
        self
    }
}

/// An editing worker's finished work. `result` is complete except for how it lands
/// (`landing`, and for review the diff page), which [`Delegator::settle`] fills in.
struct Pending {
    result: DelegateResult,
    patch: String,
    verdict: Verdict,
}

/// How one attempt ended without a result.
enum AttemptOutcome {
    /// Nothing more to try (no worker, no worktree, unreadable changes).
    Stop(String),
    /// The worker failed; [`Runtime::react`] says whether another model may try.
    Failed(AttemptFailure),
}

/// Everything one log row needs.
struct AttemptLog<'a> {
    spec: &'static ModelSpec,
    id: &'a str,
    task: TaskType,
    attempt: u32,
    read_only: bool,
    jev: &'a Value,
    result: &'a Result<TurnOutcome, TurnError>,
    wall_ms: u64,
    /// Unix ms when the backend turn began.
    started_ms: u64,
    files: &'a [String],
    tools: &'a [String],
    /// Where the work went ([`Landing::tag`], or [`Verdict::tag`] before it lands); `None`
    /// for a failed attempt.
    landing: Option<&'static str>,
    /// A read-only worker that ran in the user's folder instead of a worktree.
    in_place: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;


    use crate::providers::domain::availability::{Availability, State};
    use crate::backend::testing::{ScriptedCli, SessionId};
    use crate::backend::{ErrorKind, Usage};
    use crate::judgment::application::judge::Security;
    use ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

    const PROVIDERS: [ProviderId; 5] = [Claude, DeepSeek, Codex, Gemini, Grok];
    const NOW: u64 = 1_760_000_000_000;

    fn providers_in(mask: u32) -> Vec<ProviderId> {
        PROVIDERS.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, p)| *p).collect()
    }

    fn signals(task: TaskType) -> BriefSignals {
        BriefSignals { task, security_p: 0.0, edit_p: 0.9, read_only: false }
    }

    struct World {
        usable: Usable,
        allow: Allowances,
        overrides: Overrides,
        util: HashMap<ProviderId, f64>,
        live: HashMap<ProviderId, usize>,
    }

    impl World {
        fn new(ready: &[ProviderId]) -> Self {
            Self {
                usable: Usable::all_of(ready.iter().copied()),
                allow: Allowances { credits: true, per_token: true },
                overrides: Overrides::default(),
                util: HashMap::new(),
                live: HashMap::new(),
            }
        }

        fn pick(&self, s: &BriefSignals) -> Result<&'static ModelSpec, NoWorker> {
            let util = |p: ProviderId| self.util.get(&p).copied();
            let live = |p: ProviderId| self.live.get(&p).copied().unwrap_or(0);
            pick_worker(
                s,
                &WorkerView {
                    usable: &self.usable,
                    allow: &self.allow,
                    overrides: &self.overrides,
                    plan_utilization: &util,
                    live: &live,
                    context_tokens: 0,
                },
            )
        }
    }

    #[test]
    fn a_worker_is_never_top_tier_never_a_very_hard_model_and_always_usable() {
        for mask in 0..(1u32 << PROVIDERS.len()) {
            let ready = providers_in(mask);
            let world = World::new(&ready);
            for task in TaskType::ALL {
                for read_only in [false, true] {
                    let s = BriefSignals { task, security_p: 0.1, edit_p: 0.1, read_only };
                    if let Ok(spec) = world.pick(&s) {
                        assert_ne!(spec.tier, Tier::Top, "{task} {ready:?}: {}", spec.id);
                        assert!(!spec.very_hard_only, "{}", spec.id);
                        assert!(ready.contains(&spec.provider), "unusable provider {} for {ready:?}", spec.provider);
                    }
                }
            }
        }
    }

    #[test]
    fn security_briefs_never_go_to_deepseek_or_grok() {
        for mask in 0..(1u32 << PROVIDERS.len()) {
            let world = World::new(&providers_in(mask));
            // Jev's task label, and Jev's security probability on an otherwise ordinary brief.
            for s in [
                signals(TaskType::Security),
                BriefSignals { security_p: 0.95, ..signals(TaskType::Feature) },
                BriefSignals { security_p: 0.95, read_only: true, edit_p: 0.0, ..signals(TaskType::Explain) },
                BriefSignals::unjudged(),
            ] {
                if let Ok(spec) = world.pick(&s) {
                    assert!(!matches!(spec.provider, DeepSeek | Grok), "{:?} -> {} {}", s, spec.provider, spec.id);
                }
            }
        }
        // With only the barred providers ready there is no worker, rather than a bad one.
        let only_barred = World::new(&[DeepSeek, Grok]);
        assert_eq!(only_barred.pick(&signals(TaskType::Security)), Err(NoWorker { task: TaskType::Security, floor: Tier::Everyday }));
    }

    #[test]
    fn code_never_drops_below_everyday_but_a_read_only_lookup_may_use_light() {
        let world = World::new(&PROVIDERS);
        for task in TaskType::ALL {
            let writes = world.pick(&signals(task)).unwrap();
            assert!(writes.tier >= Tier::Everyday, "{task}: {}", writes.id);
            // The planner calling it read-only is not enough when Jev says it edits.
            let claims = BriefSignals { read_only: true, edit_p: 0.9, ..signals(task) };
            assert!(world.pick(&claims).unwrap().tier >= Tier::Everyday, "{task}");
        }
        let lookup = BriefSignals { read_only: true, edit_p: 0.05, ..signals(TaskType::Explain) };
        assert_eq!(world.pick(&lookup).unwrap().tier, Tier::Light, "Explain's chain starts with a Light model");
    }

    #[test]
    fn an_all_top_chain_falls_back_to_the_next_best_worker() {
        // Review's chain is Top models only; a review brief still gets an Everyday worker.
        let world = World::new(&[Claude]);
        assert_eq!(world.pick(&signals(TaskType::Review)).unwrap().id, "claude-sonnet-5-5");
    }

    #[test]
    fn chain_order_wins_until_a_provider_is_much_busier() {
        let mut world = World::new(&[Claude, Codex]);
        let first = world.pick(&signals(TaskType::Feature)).unwrap();
        assert_eq!(first.provider, Claude, "Feature's chain starts with Sonnet");
        world.util.insert(Claude, 0.05);
        assert_eq!(world.pick(&signals(TaskType::Feature)).unwrap().provider, Claude, "within the margin");
        world.util.insert(Claude, 0.60);
        assert_eq!(world.pick(&signals(TaskType::Feature)).unwrap().provider, Codex, "claude's plan is much busier");
    }

    #[test]
    fn parallel_workers_fan_out_across_providers() {
        let mut world = World::new(&[Claude, Codex]);
        world.live.insert(Claude, 1);
        assert_eq!(world.pick(&signals(TaskType::Feature)).unwrap().provider, Codex, "one worker already on claude");
        world.live.insert(Codex, 1);
        assert_eq!(world.pick(&signals(TaskType::Feature)).unwrap().provider, Claude, "tied: chain order again");
    }

    #[test]
    fn spreading_never_picks_a_model_the_rules_bar() {
        // DeepSeek is idle and Claude is busy, but security still cannot use DeepSeek.
        let mut world = World::new(&[Claude, DeepSeek]);
        world.util.insert(Claude, 0.95);
        assert_eq!(world.pick(&signals(TaskType::Security)).unwrap().provider, Claude);
    }

    #[test]
    fn jev_unavailable_is_treated_as_security_and_as_an_edit() {
        let u = BriefSignals::unjudged();
        assert_eq!(worker_task(&u), TaskType::Security);
        assert_eq!(worker_floor(&u), Tier::Everyday);
    }

    #[test]
    fn patch_paths_lists_each_touched_file_once() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-x\n+y\n\
                     diff --git a/old.txt b/new.txt\nrename from old.txt\nrename to new.txt\n\
                     diff --git a/src/a.rs b/src/a.rs\n";
        assert_eq!(patch_paths(patch), ["src/a.rs", "old.txt", "new.txt"]);
        assert!(patch_paths("").is_empty());
    }

    #[test]
    fn long_text_is_cut_with_a_note() {
        assert_eq!(truncate_with_note("short", 10, "diff"), "short");
        let cut = truncate_with_note(&"x".repeat(30), 10, "diff");
        assert!(cut.starts_with(&"x".repeat(10)) && cut.contains("diff truncated: 20 more characters"), "{cut}");
    }

    #[test]
    fn the_worker_prompt_confines_the_worker_and_asks_for_a_short_report() {
        let p = worker_prompt("add a retry", &["src/net.rs".into()], false);
        assert!(p.contains("only inside the current directory") && p.contains("do not use the network"));
        assert!(p.contains("short report") && p.contains("add a retry") && p.contains("src/net.rs"));
        assert!(p.contains("Run the project's tests") && p.contains("merged only if they pass"));
        assert!(!worker_prompt("x", &[], false).contains("Files that are probably involved"));
        // A read-only worker is told to change nothing, and nothing about merging.
        let r = worker_prompt("find the callers", &[], true);
        assert!(r.contains("READ-ONLY") && r.contains("do not create, change or delete any file") && r.contains("find the callers"));
        assert!(!r.contains("merged") && !r.contains("worktree"));
    }

    #[test]
    fn only_clis_with_a_real_plan_mode_read_in_place() {
        let in_place: Vec<ProviderId> = PROVIDERS.into_iter().filter(|p| plan_mode_enforced(*p)).collect();
        assert_eq!(in_place, [Claude, DeepSeek, Codex]);
    }

    // ---------------------------------------------------------------- the Delegator

    /// A "CLI" that is a shell script with canned lines: `FIN:text`, `FAIL:message`.
    /// `started` gets (provider, model, dir, prompt) and `modes` the permission mode of every
    /// turn started, in order.
    fn fake_cli(provider: ProviderId, script: &'static str, started: &Started, modes: &Modes) -> ScriptedCli {
        let (started, modes) = (started.clone(), modes.clone());
        ScriptedCli::new(provider, script).session_id(SessionId::Absent).on_start(move |req| {
            started.lock().unwrap().push((provider, req.model.clone(), req.dir.clone(), req.prompt.clone()));
            modes.lock().unwrap().push(req.mode);
        })
    }

    type Modes = Arc<Mutex<Vec<Mode>>>;

    const OK: &str = "printf 'TOOL:Edit a.rs\\nFIN:did the piece\\n'";
    const QUOTA: &str = "printf 'FAIL:usage limit reached|1794853451\\n'";

    /// Directories made and removed, and what `apply` was given; no git involved.
    #[derive(Default)]
    struct FakeWorktrees {
        base: PathBuf,
        created: Mutex<Vec<PathBuf>>,
        removed: Mutex<Vec<PathBuf>>,
        applied: Mutex<Vec<String>>,
        unapplied: Mutex<Vec<String>>,
        pruned: Mutex<Vec<PathBuf>>,
        apply_fails: bool,
        unapply_fails: bool,
        create_fails: bool,
        /// Every `create` and `apply` takes this long, like a big checkout would.
        delay: Duration,
        /// What `diff` returns for every worker; by default a one-file patch named after it.
        patch: Mutex<Option<String>>,
    }

    fn temp_dir(tag: &str) -> PathBuf {
        crate::testkit::scratch_dir::unique_scratch_dir("delegate", tag)
    }

    impl FakeWorktrees {
        fn new() -> Arc<Self> {
            let base = temp_dir("fake");
            // The "main tree", where a read-only worker runs in place.
            std::fs::create_dir_all(base.join("repo")).unwrap();
            Arc::new(Self { base, ..Self::default() })
        }

        fn slow(delay: Duration) -> Arc<Self> {
            Arc::new(Self { base: temp_dir("slow"), delay, ..Self::default() })
        }

        fn with_patch(self: Arc<Self>, patch: String) -> Arc<Self> {
            *self.patch.lock().unwrap() = Some(patch);
            self
        }
    }

    impl Worktrees for Arc<FakeWorktrees> {
        fn repo(&self) -> PathBuf {
            self.base.join("repo")
        }
        fn root(&self) -> PathBuf {
            self.base.clone()
        }
        fn create(&self, id: &str) -> Result<PathBuf> {
            std::thread::sleep(self.delay);
            if self.create_fails {
                bail!("disk full");
            }
            let p = self.base.join(id);
            std::fs::create_dir_all(&p)?;
            self.created.lock().unwrap().push(p.clone());
            Ok(p)
        }
        fn diff(&self, path: &Path) -> Result<String> {
            if let Some(p) = self.patch.lock().unwrap().clone() {
                return Ok(p);
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            Ok(file_patch(&format!("{name}.rs"), "a", &["b"]))
        }
        fn apply(&self, patch: &str) -> Result<()> {
            std::thread::sleep(self.delay);
            if self.apply_fails {
                bail!("conflict");
            }
            self.applied.lock().unwrap().push(patch.to_string());
            Ok(())
        }
        fn unapply(&self, patch: &str) -> Result<()> {
            if self.unapply_fails {
                bail!("those files changed since");
            }
            self.unapplied.lock().unwrap().push(patch.to_string());
            Ok(())
        }
        fn remove(&self, path: &Path) -> Result<()> {
            self.removed.lock().unwrap().push(path.to_path_buf());
            let _ = std::fs::remove_dir_all(path);
            Ok(())
        }
        fn prune(&self, repo: &Path) -> Result<()> {
            self.pruned.lock().unwrap().push(repo.to_path_buf());
            Ok(())
        }
    }

    /// A project whose test command is `command` (none: no tests known), and whose every run
    /// ends as `outcome`. Records where it ran.
    struct FakeChecks {
        command: Option<&'static str>,
        outcome: CheckRun,
        ran: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl Checks for FakeChecks {
        fn test_command(&self, _dir: &Path) -> Option<String> {
            self.command.map(String::from)
        }
        fn run(&self, dir: &Path, _command: &str, _stop: &dyn Fn() -> bool) -> CheckRun {
            self.ran.lock().unwrap().push(dir.to_path_buf());
            self.outcome.clone()
        }
    }

    /// `rig`'s delegator with a project whose `cargo test` ends as `outcome`; where it ran.
    fn tested(mut r: Rig, outcome: CheckRun) -> (Rig, Arc<Mutex<Vec<PathBuf>>>) {
        let ran = Arc::new(Mutex::new(Vec::new()));
        r.d = r.d.with_checks(Box::new(FakeChecks { command: Some("cargo test"), outcome, ran: ran.clone() }));
        (r, ran)
    }

    /// A one-file patch: `old` replaced by `new` lines.
    fn file_patch(path: &str, old: &str, new: &[&str]) -> String {
        let added: String = new.iter().map(|l| format!("+{l}\n")).collect();
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1,{} @@\n-{old}\n{added}", new.len())
    }

    /// A one-file patch of about `chars` characters.
    fn big_patch(path: &str, chars: usize) -> String {
        let line = "x".repeat(99);
        file_patch(path, "a", &vec![line.as_str(); chars / 100])
    }

    fn want(task: TaskType) -> ProviderWant {
        ProviderWant {
            task,
            task_confidence: 0.9,
            very_hard_p: 0.0,
            security_p: 0.0,
            edit_p: 0.9,
            continues_p: 0.0,
            correction_p: None,
            orchestration: crate::judgment::application::judge::Orchestration::default(),
            security_keywords: Security::None,
            signals: json!({ "task": task.as_str() }),
        }
    }

    fn availability(provider: ProviderId, ready: bool) -> Availability {
        let state = if ready { State::Ready } else { State::NotLoggedIn };
        Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
    }

    type Started = Arc<Mutex<Vec<(ProviderId, String, PathBuf, String)>>>;
    type Scripts = Arc<Mutex<HashMap<ProviderId, &'static str>>>;

    struct Rig {
        d: Delegator,
        wt: Arc<FakeWorktrees>,
        started: Started,
        scripts: Scripts,
        modes: Modes,
    }

    /// A delegator on `wt` whose fake CLIs all succeed unless `scripts` says otherwise.
    fn delegator_on(wt: &Arc<FakeWorktrees>, ready: &[ProviderId], task: TaskType, started: &Started, scripts: &Scripts, modes: &Modes) -> Delegator {
        let avail = PROVIDERS.iter().map(|p| availability(*p, ready.contains(p))).collect();
        let (st, sc, md) = (started.clone(), scripts.clone(), modes.clone());
        let judge: Judge = Box::new(move |_| Box::pin(async move { Some(want(task)) }));
        Delegator::new(Runtime::for_tests(avail), judge, Box::new(wt.clone()))
            .with_backend_factory(Box::new(move |p| {
                let script = sc.lock().unwrap()[&p];
                Box::new(fake_cli(p, script, &st, &md))
            }))
            .with_clock(Box::new(|| NOW))
            .with_idle_timeout(Duration::from_millis(500))
            .with_planner("claude:claude-opus-5-5".into())
            // No test command known: work goes to review unless a test says otherwise.
            .with_checks(Box::new(FakeChecks { command: None, outcome: CheckRun::Passed, ran: Arc::default() }))
    }

    fn rig_on(wt: Arc<FakeWorktrees>, ready: &[ProviderId], task: TaskType) -> Rig {
        let started: Started = Arc::new(Mutex::new(Vec::new()));
        let scripts: Scripts = Arc::new(Mutex::new(PROVIDERS.iter().map(|p| (*p, OK)).collect()));
        let modes = Modes::default();
        let d = delegator_on(&wt, ready, task, &started, &scripts, &modes);
        Rig { d, wt, started, scripts, modes }
    }

    fn rig(ready: &[ProviderId], task: TaskType) -> Rig {
        rig_on(FakeWorktrees::new(), ready, task)
    }

    fn brief(text: &str) -> DelegateRequest {
        DelegateRequest { brief: text.into(), ..DelegateRequest::default() }
    }

    #[tokio::test]
    async fn a_worker_runs_in_its_own_worktree_and_reports_summary_and_diff() {
        let r = rig(&[Claude, Codex], TaskType::Feature);
        let res = r.d.delegate(DelegateRequest { files: vec!["src/net.rs".into()], ..brief("add a retry to the client") }).await;
        assert!(res.ok, "{:?}", res.error);
        assert_eq!(res.provider, Some(Claude));
        assert_eq!(res.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(res.summary, "did the piece");
        assert_eq!(res.files_changed(), [format!("{}.rs", res.worker_id)]);
        assert_eq!(res.stats[0].render(), format!("{}.rs +1 -1", res.worker_id));
        assert_eq!(res.shown, res.files_changed(), "a small diff is shown in full");
        assert!(res.diff.contains("+b"));
        let started = r.started.lock().unwrap().clone();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].2, r.wt.base.join(&res.worker_id), "the worker ran in its worktree");
        assert!(started[0].3.contains("add a retry to the client") && started[0].3.contains("src/net.rs"));
        assert!(res.render().contains(&format!("merge({})", res.worker_id)));
    }

    #[tokio::test]
    async fn two_delegations_get_two_separate_directories() {
        let r = rig(&[Claude, Codex], TaskType::Feature);
        let (a, b) = tokio::join!(r.d.delegate(brief("piece one")), r.d.delegate(brief("piece two")));
        assert!(a.ok && b.ok);
        assert_ne!(a.worker_id, b.worker_id);
        let dirs: HashSet<_> = r.started.lock().unwrap().iter().map(|s| s.2.clone()).collect();
        assert_eq!(dirs.len(), 2, "workers never share a directory");
        // Run together, the second pick sees the first as live and goes to the other provider.
        let providers: HashSet<_> = r.started.lock().unwrap().iter().map(|s| s.0).collect();
        assert_eq!(providers.len(), 2, "parallel workers spread across providers");
    }

    #[tokio::test]
    async fn quota_retries_once_on_a_different_provider_and_cools_the_first_down() {
        let r = rig(&[Claude, Codex], TaskType::Feature);
        r.scripts.lock().unwrap().insert(Claude, QUOTA);
        let res = r.d.delegate(brief("add a retry")).await;
        assert!(res.ok, "{:?}", res.error);
        assert_eq!(res.provider, Some(Codex), "Sonnet hit its limit; the next worker is another provider");
        assert_eq!(res.model.as_deref(), Some("gpt-6.1-terra"));
        let started = r.started.lock().unwrap().clone();
        assert_eq!(started.iter().map(|s| s.0).collect::<Vec<_>>(), [Claude, Codex]);
        assert_ne!(started[0].1, started[1].1);
        assert_eq!(res.retries.len(), 1);
        assert!(res.retries[0].contains("hit its limit"), "{:?}", res.retries);
        assert!(lock(&r.d.runtime).cooldowns.active(Claude, NOW).is_some());
        assert_eq!(r.wt.removed.lock().unwrap().len(), 1, "the failed worker's worktree is gone");
        assert_eq!(r.wt.created.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_second_failure_ends_the_delegation_instead_of_looping() {
        let r = rig(&[Claude, Codex], TaskType::Feature);
        r.scripts.lock().unwrap().insert(Claude, QUOTA);
        r.scripts.lock().unwrap().insert(Codex, QUOTA);
        let res = r.d.delegate(brief("add a retry")).await;
        assert!(!res.ok);
        assert_eq!(r.started.lock().unwrap().len(), 2, "one retry, no more");
        assert!(res.error.unwrap().contains("failed"));
    }

    #[tokio::test]
    async fn an_unusable_provider_never_starts_and_no_worker_is_reported_plainly() {
        let r = rig(&[Grok, DeepSeek], TaskType::Security);
        let res = r.d.delegate(brief("audit the login route")).await;
        assert!(!res.ok);
        assert!(r.started.lock().unwrap().is_empty(), "nothing was started");
        assert!(res.error.unwrap().contains("no usable worker model"));
        // And a worker only ever starts on a ready provider.
        let r = rig(&[Codex], TaskType::Feature);
        r.d.delegate(brief("x")).await;
        assert!(r.started.lock().unwrap().iter().all(|s| s.0 == Codex));
    }

    #[tokio::test]
    async fn a_failure_no_other_provider_can_fix_stops_at_once() {
        let r = rig(&[Claude, Codex], TaskType::Feature);
        r.scripts.lock().unwrap().insert(Claude, "printf 'FAIL:segmentation fault\\n'");
        let res = r.d.delegate(brief("x")).await;
        assert!(!res.ok);
        assert_eq!(r.started.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn merge_applies_the_workers_diff_and_removes_its_worktree() {
        let r = rig(&[Claude], TaskType::Feature);
        let res = r.d.delegate(brief("x")).await;
        let msg = r.d.merge(&res.worker_id, None).await.unwrap();
        assert!(msg.contains("merged worker") && msg.contains(&res.worker_id));
        assert_eq!(r.wt.applied.lock().unwrap().len(), 1);
        assert_eq!(r.wt.removed.lock().unwrap().len(), 1);
        assert!(r.d.merge(&res.worker_id, None).await.unwrap_err().contains("unknown worker id"), "a worker merges once");
    }

    #[tokio::test]
    async fn a_failed_merge_keeps_the_worktree_for_discard() {
        let wt = Arc::new(FakeWorktrees { base: temp_dir("conflict"), apply_fails: true, ..FakeWorktrees::default() });
        let avail = PROVIDERS.iter().map(|p| availability(*p, *p == Claude)).collect();
        let judge: Judge = Box::new(|_| Box::pin(async { Some(want(TaskType::Feature)) }));
        let st = Arc::new(Mutex::new(Vec::new()));
        let d = Delegator::new(Runtime::for_tests(avail), judge, Box::new(wt.clone()))
            .with_backend_factory(Box::new(move |p| Box::new(fake_cli(p, OK, &st, &Modes::default()))))
            .with_idle_timeout(Duration::from_millis(500));
        let res = d.delegate(brief("x")).await;
        let err = d.merge(&res.worker_id, None).await.unwrap_err();
        assert!(err.contains("conflict") && err.contains("discard"), "{err}");
        assert!(wt.removed.lock().unwrap().is_empty());
        assert!(d.discard(&res.worker_id).await.unwrap().contains("discarded"));
        assert_eq!(wt.removed.lock().unwrap().len(), 1);
        assert!(d.discard(&res.worker_id).await.is_err());
    }

    // ---------------------------------------------------------------- auto-merge

    #[tokio::test]
    async fn work_whose_tests_pass_merges_itself_and_the_planner_gets_no_diff() {
        let (r, ran) = tested(rig(&[Claude], TaskType::Feature), CheckRun::Passed);
        let res = r.d.delegate(brief("add a retry")).await;
        assert!(res.ok, "{:?}", res.error);
        assert_eq!(res.landing, Landing::AutoMerged { command: "cargo test".into() });
        assert_eq!(*ran.lock().unwrap(), [r.wt.base.join(&res.worker_id)], "the tests ran in the worker's worktree");
        assert_eq!(r.wt.applied.lock().unwrap().len(), 1, "applied to the main tree");
        assert_eq!(*r.wt.removed.lock().unwrap(), [r.wt.base.join(&res.worker_id)], "its worktree is gone");
        assert!(lock(&r.d.live).is_empty(), "nothing is left to merge or discard");
        assert!(res.diff.is_empty() && res.shown.is_empty());
        let text = res.render();
        assert!(text.contains("done, auto-merged") && text.contains(&format!("undo({})", res.worker_id)) && text.contains("`cargo test` passed"), "{text}");
        assert!(text.contains(&format!("{}.rs +1 -1", res.worker_id)) && text.contains("did the piece"), "the file list and report still come back: {text}");
        assert!(!text.contains("diff (full diffs") && !text.contains("merge("), "{text}");
    }

    #[tokio::test]
    async fn undo_takes_exactly_the_merged_patch_back_out_once() {
        let (r, _) = tested(rig(&[Claude], TaskType::Feature), CheckRun::Passed);
        let res = r.d.delegate(brief("x")).await;
        let msg = r.d.undo(&res.worker_id).await.unwrap();
        assert!(msg.contains(&format!("{}.rs", res.worker_id)) && msg.contains("back to how they were"), "{msg}");
        assert_eq!(*r.wt.unapplied.lock().unwrap(), *r.wt.applied.lock().unwrap(), "the same patch, reversed");
        assert!(r.d.undo(&res.worker_id).await.unwrap_err().contains("no auto-merged worker"), "an undo happens once");
        assert!(r.d.undo("w-never").await.is_err());
    }

    #[tokio::test]
    async fn an_undo_that_does_not_apply_keeps_the_record_to_try_again() {
        let wt = Arc::new(FakeWorktrees { base: temp_dir("noundo"), unapply_fails: true, ..FakeWorktrees::default() });
        let (r, _) = tested(rig_on(wt, &[Claude], TaskType::Feature), CheckRun::Passed);
        let id = r.d.delegate(brief("x")).await.worker_id;
        let err = r.d.undo(&id).await.unwrap_err();
        assert!(err.contains("could not undo") && err.contains("changed since"), "{err}");
        assert!(lock(&r.d.merged).contains_key(&id));
    }

    #[tokio::test]
    async fn failing_tests_send_the_work_to_review_with_their_output() {
        let (r, _) = tested(rig(&[Claude], TaskType::Feature), CheckRun::Failed { output: "test net::retry ... FAILED".into() });
        let res = r.d.delegate(brief("x")).await;
        assert!(matches!(&res.landing, Landing::Review(ReviewReason::TestsFailed { command, .. }) if command == "cargo test"), "{:?}", res.landing);
        assert!(r.wt.applied.lock().unwrap().is_empty() && r.wt.removed.lock().unwrap().is_empty(), "nothing applied, worktree kept");
        let text = res.render();
        assert!(text.contains("waiting for your review") && text.contains("Not auto-merged: `cargo test` failed") && text.contains("net::retry ... FAILED"), "{text}");
        assert!(text.contains("diff (full diffs") && text.contains(&format!("merge({})", res.worker_id)), "the review flow as before: {text}");
        assert!(r.d.merge(&res.worker_id, None).await.is_ok(), "the planner can still merge it");
    }

    #[tokio::test]
    async fn every_other_reason_falls_back_to_review_without_touching_the_main_tree() {
        // No test command known (the rig's default).
        let r = rig(&[Claude], TaskType::Feature);
        assert_eq!(r.d.delegate(brief("x")).await.landing, Landing::Review(ReviewReason::NoTestCommand));
        // The planner asked to review: the tests do not even run.
        let (r, ran) = tested(rig(&[Claude], TaskType::Feature), CheckRun::Passed);
        assert_eq!(r.d.delegate(DelegateRequest { review: true, ..brief("x") }).await.landing, Landing::Review(ReviewReason::Asked));
        assert!(ran.lock().unwrap().is_empty());
        // Auto-merge switched off.
        let (mut r, _) = tested(rig(&[Claude], TaskType::Feature), CheckRun::Passed);
        r.d = r.d.with_auto_merge(false);
        assert_eq!(r.d.delegate(brief("x")).await.landing, Landing::Review(ReviewReason::Off));
        // Tests too slow, or not runnable.
        let (r, _) = tested(rig(&[Claude], TaskType::Feature), CheckRun::TimedOut { after: Duration::from_secs(900) });
        let res = r.d.delegate(brief("x")).await;
        assert!(res.render().contains("ran longer than 15 minutes"), "{}", res.render());
        let (r, _) = tested(rig(&[Claude], TaskType::Feature), CheckRun::CouldNotStart { error: "no sh".into() });
        assert!(matches!(r.d.delegate(brief("x")).await.landing, Landing::Review(ReviewReason::TestsNotRun { .. })));
        assert!(r.wt.applied.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tests_that_pass_on_a_patch_that_conflicts_go_to_review() {
        let wt = Arc::new(FakeWorktrees { base: temp_dir("conflict-auto"), apply_fails: true, ..FakeWorktrees::default() });
        let (r, _) = tested(rig_on(wt, &[Claude], TaskType::Feature), CheckRun::Passed);
        let res = r.d.delegate(brief("x")).await;
        assert!(matches!(&res.landing, Landing::Review(ReviewReason::Conflict(why)) if why.contains("conflict")), "{:?}", res.landing);
        assert!(r.wt.removed.lock().unwrap().is_empty(), "the worktree is kept for review");
        assert!(lock(&r.d.merged).is_empty(), "nothing to undo");
        assert!(res.render().contains("the tests passed, but the changes do not apply cleanly"));
    }

    #[tokio::test]
    async fn a_worker_that_changed_nothing_leaves_nothing_behind() {
        let (r, ran) = tested(rig_on(FakeWorktrees::new().with_patch(String::new()), &[Claude], TaskType::Feature), CheckRun::Passed);
        let res = r.d.delegate(brief("x")).await;
        assert_eq!(res.landing, Landing::NoChanges);
        assert!(ran.lock().unwrap().is_empty() && r.wt.applied.lock().unwrap().is_empty());
        assert_eq!(r.wt.removed.lock().unwrap().len(), 1);
        assert!(lock(&r.d.live).is_empty());
        assert!(res.render().contains("files changed: none"));
    }

    // ---------------------------------------------------------------- background tasks

    const SLOW: &str = "sleep 0.4; printf 'FIN:did the piece\\n'";

    /// Until task `id` has ended, without collecting it.
    async fn ended(d: &Delegator, id: &str) {
        let mut rx = lock(&d.tasks)[&task_number(id).unwrap()].done.clone();
        let _ = rx.wait_for(|done| *done).await;
    }

    #[tokio::test]
    async fn start_returns_at_once_and_the_work_lands_only_when_waited_for() {
        let (r, _) = tested(rig(&[Claude], TaskType::Feature), CheckRun::Passed);
        r.scripts.lock().unwrap().insert(Claude, SLOW);
        let (d, wt) = (Arc::new(r.d), r.wt);
        let begun = Instant::now();
        let id = d.start(brief("add a retry"));
        assert_eq!(id, "t1");
        assert!(begun.elapsed() < Duration::from_millis(200), "start does not wait for the worker");
        ended(&d, &id).await;
        assert!(wt.applied.lock().unwrap().is_empty(), "tested, but nothing lands while the planner may be editing");
        let text = d.wait(None, None).await.unwrap();
        assert!(text.starts_with("task t1: worker ") && text.contains("done, auto-merged"), "{text}");
        assert_eq!(wt.applied.lock().unwrap().len(), 1, "landed by wait");
        assert!(d.wait(None, None).await.unwrap().contains("every task has been collected"), "each task is collected once");
        assert!(d.wait(Some(vec!["t1".into()]), None).await.unwrap_err().contains("unknown task `t1`"));
    }

    #[tokio::test]
    async fn parallel_tasks_fan_out_and_one_wait_collects_them_all() {
        let r = rig_on(FakeWorktrees::slow(Duration::from_millis(100)), &[Claude, Codex], TaskType::Feature);
        let d = Arc::new(r.d);
        let (a, b) = (d.start(brief("one")), d.start(brief("two")));
        assert_ne!(a, b);
        let text = d.wait(None, None).await.unwrap();
        assert!(text.contains("task t1: worker") && text.contains("task t2: worker"), "{text}");
        let providers: HashSet<_> = r.started.lock().unwrap().iter().map(|s| s.0).collect();
        assert_eq!(providers.len(), 2, "the second pick saw the first as running");
    }

    #[tokio::test]
    async fn a_timed_wait_reports_what_ended_and_lists_the_rest() {
        let r = rig(&[Claude, Codex], TaskType::Feature);
        r.scripts.lock().unwrap().insert(Codex, SLOW);
        let d = Arc::new(r.d);
        // Spread across providers, the first goes to Claude (fast), the second to Codex (slow).
        let (fast, slow) = (d.start(brief("one")), d.start(brief("two")));
        ended(&d, &fast).await;
        let text = d.wait(None, Some(Duration::ZERO)).await.unwrap();
        assert!(text.contains(&format!("task {fast}: worker")), "{text}");
        assert!(text.contains(&format!("Still running after 0ms: {slow} (running for ")) && text.contains("Call wait again"), "{text}");
        let text = d.wait(Some(vec![slow.clone(), slow.clone()]), None).await.unwrap();
        assert!(text.starts_with(&format!("task {slow}: worker")) && !text.contains("Still running"), "named once, collected once: {text}");
    }

    #[tokio::test]
    async fn wait_names_unknown_tasks_and_refuses_an_empty_list() {
        let d = Arc::new(rig(&[Claude], TaskType::Feature).d);
        assert!(d.wait(None, None).await.unwrap().contains("every task has been collected"));
        let open = d.start(brief("x"));
        let err = d.wait(Some(vec!["t99".into()]), None).await.unwrap_err();
        assert!(err.contains("unknown task `t99`") && err.contains(&format!("uncollected tasks: {open}")), "{err}");
        assert!(d.wait(Some(vec!["nonsense".into()]), None).await.is_err());
        assert!(d.wait(Some(vec![]), None).await.unwrap_err().contains("`task_ids` is empty"));
        assert!(d.wait(None, None).await.unwrap().contains("task t1"));
    }

    #[tokio::test]
    async fn a_task_that_dies_is_reported_as_failed() {
        let d = Arc::new(rig(&[Claude], TaskType::Feature).d);
        // A sender dropped without an outcome is how a panicked task looks.
        let (tx, done) = tokio::sync::watch::channel(false);
        lock(&d.tasks).insert(7, Task { started_ms: NOW, done, outcome: Arc::default() });
        drop(tx);
        let text = d.wait(None, None).await.unwrap();
        assert!(text.starts_with("task t7: ") && text.contains("(no worker): FAILED") && text.contains("stopped unexpectedly"), "{text}");
    }

    // ---------------------------------------------------------------- read-only workers

    #[tokio::test]
    async fn a_read_only_worker_reads_the_main_tree_in_plan_mode_with_no_worktree() {
        let (r, ran) = tested(rig(&[Claude], TaskType::Feature), CheckRun::Passed);
        let res = r.d.delegate(DelegateRequest { read_only: true, ..brief("find the callers of retry") }).await;
        assert!(res.ok, "{:?}", res.error);
        assert_eq!(res.landing, Landing::Report { discarded: vec![] });
        let started = r.started.lock().unwrap().clone();
        assert_eq!(started[0].2, r.wt.repo(), "it ran in the user's folder");
        assert!(started[0].3.contains("READ-ONLY"));
        assert_eq!(*r.modes.lock().unwrap(), [Mode::Plan]);
        assert!(r.wt.created.lock().unwrap().is_empty() && r.wt.removed.lock().unwrap().is_empty(), "no worktree");
        assert!(ran.lock().unwrap().is_empty() && r.wt.applied.lock().unwrap().is_empty());
        assert!(lock(&r.d.live).is_empty(), "nothing to merge or discard");
        let text = res.render();
        assert!(text.contains("done (read-only)") && text.contains("did the piece") && !text.contains("merge(") && !text.contains("files changed"), "{text}");
        // An editing worker still runs in the default mode.
        r.d.delegate(brief("x")).await;
        assert_eq!(r.modes.lock().unwrap()[1], Mode::default());
    }

    #[tokio::test]
    async fn a_read_only_worker_on_a_cli_without_real_plan_mode_gets_a_throwaway_worktree() {
        let r = rig(&[Gemini], TaskType::Feature);
        let res = r.d.delegate(DelegateRequest { read_only: true, ..brief("find the callers") }).await;
        assert!(res.ok, "{:?}", res.error);
        assert_eq!(res.provider, Some(Gemini));
        let id = &res.worker_id;
        assert_eq!(r.started.lock().unwrap()[0].2, r.wt.base.join(id), "not the user's folder");
        assert_eq!(*r.modes.lock().unwrap(), [Mode::Plan], "asked for plan mode anyway");
        // The fake worker "changed" a file: it is reported and thrown away, never merged.
        assert_eq!(res.landing, Landing::Report { discarded: vec![format!("{id}.rs")] });
        assert_eq!(*r.wt.removed.lock().unwrap(), [r.wt.base.join(id)]);
        assert!(r.wt.applied.lock().unwrap().is_empty() && lock(&r.d.live).is_empty());
        assert!(res.render().contains("they were thrown away"), "{}", res.render());
    }

    #[tokio::test]
    async fn each_attempt_is_logged_with_planner_worker_signals_and_files() {
        let log_path = temp_dir("log").join("orchestration.jsonl");
        let mut r = rig(&[Claude, Codex], TaskType::Feature);
        r.d = r.d.with_log(Some(log_path.to_string_lossy().into_owned()));
        r.scripts.lock().unwrap().insert(Claude, QUOTA);
        r.d.delegate(brief("add a retry")).await;
        let rows: Vec<Value> = std::fs::read_to_string(&log_path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"], "delegate");
        assert_eq!(rows[0]["planner"], "claude:claude-opus-5-5");
        assert_eq!((rows[0]["ok"].clone(), rows[0]["error_kind"].clone()), (json!(false), json!("quota")));
        assert_eq!(rows[1]["provider"], "codex");
        assert_eq!(rows[1]["ok"], true);
        assert_eq!(rows[1]["attempt"], 2);
        assert_eq!(rows[1]["signals"]["task"], "feature");
        assert_eq!(rows[1]["usage"]["output_tokens"], 7);
        assert_eq!(rows[1]["files_changed"].as_array().unwrap().len(), 1);
        assert!(rows[1]["wall_ms"].is_u64());
        for row in &rows {
            let (start, end) = (row["started_at"].as_u64().unwrap(), row["finished_at"].as_u64().unwrap());
            assert!(end >= start && start > 0);
            assert_eq!(row["duration_ms"], row["wall_ms"]);
            assert_eq!(row["in_place"], false);
        }
        assert_eq!((rows[0]["landing"].clone(), rows[1]["landing"].clone()), (Value::Null, json!("review_no_tests")), "how the work landed");
    }

    // ---------------------------------------------------------------- real git

    fn run_git(dir: &Path, args: &[&str]) -> String {
        git(dir, args, None).unwrap_or_else(|e| panic!("{e:#}"))
    }

    /// A repository with one commit holding `f.txt` (five lines).
    fn temp_repo() -> PathBuf {
        let d = temp_dir("repo");
        crate::testkit::git_repo::init_with_commit(&d, &[("f.txt", "a\nb\nc\nd\ne\n")]);
        d
    }

    #[test]
    fn real_worktrees_are_isolated_and_diff_includes_new_files() {
        let repo = temp_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let (a, b) = (wt.create("wa").unwrap(), wt.create("wb").unwrap());
        assert_ne!(a, b);
        std::fs::write(a.join("f.txt"), "a\nB-from-a\nc\nd\ne\n").unwrap();
        std::fs::write(b.join("new.txt"), "hello\n").unwrap();
        assert_eq!(std::fs::read_to_string(b.join("f.txt")).unwrap(), "a\nb\nc\nd\ne\n", "b does not see a's edit");
        assert!(!a.join("new.txt").exists(), "a does not see b's file");
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "a\nb\nc\nd\ne\n", "the main tree is untouched");
        let (da, db) = (wt.diff(&a).unwrap(), wt.diff(&b).unwrap());
        assert_eq!(patch_paths(&da), ["f.txt"]);
        assert_eq!(patch_paths(&db), ["new.txt"]);
        wt.remove(&a).unwrap();
        assert!(!a.exists() && b.exists());
    }

    #[test]
    fn real_merge_applies_a_workers_diff_to_the_main_tree() {
        let repo = temp_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let w = wt.create("w1").unwrap();
        std::fs::write(w.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
        std::fs::write(w.join("new.txt"), "hello\n").unwrap();
        // The planner has since changed another part of the same file: a 3-way merge keeps both.
        std::fs::write(repo.join("f.txt"), "a\nb\nc\nd\nE-from-planner\n").unwrap();
        run_git(&repo, &["add", "f.txt"]);
        wt.apply(&wt.diff(&w).unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "a\nB-from-worker\nc\nd\nE-from-planner\n");
        assert_eq!(std::fs::read_to_string(repo.join("new.txt")).unwrap(), "hello\n");
    }

    #[test]
    fn real_undo_takes_a_merge_back_out_plain_or_three_way() {
        let repo = temp_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        // A plain merge: the working tree changes, the index does not.
        let w = wt.create("w1").unwrap();
        std::fs::write(w.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
        std::fs::write(w.join("new.txt"), "hello\n").unwrap();
        let patch = wt.diff(&w).unwrap();
        wt.apply(&patch).unwrap();
        wt.unapply(&patch).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "a\nb\nc\nd\ne\n");
        assert!(!repo.join("new.txt").exists(), "a file the worker created is gone again");
        assert_eq!(run_git(&repo, &["status", "--porcelain"]), "", "the tree is clean again");
        // A 3-way merge (the planner staged an edit to the same file) stages the result; undo puts the index back too.
        std::fs::write(repo.join("f.txt"), "a\nb\nc\nd\nE-from-planner\n").unwrap();
        run_git(&repo, &["add", "f.txt"]);
        let w2 = wt.create("w2").unwrap();
        std::fs::write(w2.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
        let patch = wt.diff(&w2).unwrap();
        wt.apply(&patch).unwrap();
        wt.unapply(&patch).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "a\nb\nc\nd\nE-from-planner\n", "the planner's edit stays");
        assert_eq!(run_git(&repo, &["status", "--porcelain"]), "M  f.txt\n", "staged as the planner left it, nothing unstaged");
        // After the files moved on, undo refuses and changes nothing.
        wt.apply(&patch).unwrap();
        std::fs::write(repo.join("f.txt"), "rewritten\n").unwrap();
        assert!(format!("{:#}", wt.unapply(&patch).unwrap_err()).contains("nothing was changed"));
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "rewritten\n");
    }

    #[test]
    fn a_conflicting_merge_leaves_the_tree_untouched() {
        let repo = temp_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let w = wt.create("w1").unwrap();
        std::fs::write(w.join("f.txt"), "a\nB-from-worker\nc\nd\ne\n").unwrap();
        std::fs::write(w.join("new.txt"), "hello\n").unwrap();
        let patch = wt.diff(&w).unwrap();
        // The planner changed the very same line.
        std::fs::write(repo.join("f.txt"), "a\nB-from-planner\nc\nd\ne\n").unwrap();
        run_git(&repo, &["add", "f.txt"]);
        let before_status = run_git(&repo, &["status", "--porcelain"]);
        let before_index = run_git(&repo, &["ls-files", "--stage"]);
        let err = wt.apply(&patch).unwrap_err();
        assert!(format!("{err:#}").contains("nothing was changed"), "{err:#}");
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "a\nB-from-planner\nc\nd\ne\n", "no conflict markers");
        assert!(!repo.join("new.txt").exists(), "no partial application");
        assert_eq!(run_git(&repo, &["status", "--porcelain"]), before_status);
        assert_eq!(run_git(&repo, &["ls-files", "--stage"]), before_index, "the index is as it was");
    }

    #[test]
    fn worktrees_need_a_git_repository() {
        let not_a_repo = temp_dir("plain");
        let err = GitWorktrees::new(&not_a_repo, temp_dir("root")).err().expect("not a repository");
        assert!(format!("{err:#}").contains("not inside a git repository"));
    }

    #[test]
    fn the_result_text_names_the_worker_and_failure() {
        let failed = DelegateResult::failed("boom".into(), vec!["claude hit its limit".into()]);
        let text = failed.render();
        assert!(text.contains("FAILED") && text.contains("error: boom") && text.contains("retry: claude hit its limit"));
    }

    // ---------------------------------------------------------------- the diff the planner sees

    #[test]
    fn a_patch_splits_per_file_and_counts_changes() {
        let patch = format!(
            "{}{}diff --git a/img.png b/img.png\nnew file mode 100644\nindex 0000000..1111111\nGIT binary patch\nliteral 3\nKcmZ?)\n\n",
            file_patch("src/a.rs", "old", &["n1", "n2"]),
            "diff --git a/old.txt b/new.txt\nsimilarity index 90%\nrename from old.txt\nrename to new.txt\n--- a/old.txt\n+++ b/new.txt\n@@ -1 +1 @@\n-x\n+y\n"
        );
        let parts = split_patch(&patch);
        assert_eq!(parts.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), ["src/a.rs", "new.txt", "img.png"]);
        assert_eq!(join_diffs(&parts), patch, "the parts add up to the patch");
        let stats: Vec<_> = parts.iter().map(FileDiff::stat).collect();
        assert_eq!((stats[0].added, stats[0].removed), (2, 1));
        assert_eq!((stats[1].added, stats[1].removed), (1, 1), "the +++ and --- header lines are not counted");
        assert!(stats[2].binary && stats[2].render() == "img.png (binary)");
        assert!(split_patch("").is_empty());
        // A path that itself contains " b/" is read whole; a quoted header keeps its raw text, so it is still listed.
        assert_eq!(split_patch("diff --git a/x b/y b/x b/y\n")[0].path, "x b/y");
        assert_eq!(split_patch("diff --git \"a/caf\\303\\251\" \"b/caf\\303\\251\"\n")[0].path, "\"a/caf\\303\\251\" \"b/caf\\303\\251\"");
    }

    #[test]
    fn the_first_page_holds_whole_files_only() {
        let parts = split_patch(&format!("{}{}{}", big_patch("big.rs", 5000), file_patch("small.rs", "a", &["b"]), big_patch("big2.rs", 100)));
        let (text, shown) = first_page(&parts, 3000);
        assert_eq!(shown, ["small.rs", "big2.rs"], "a file that does not fit is skipped, a later small one still goes in");
        assert!(text.contains("small.rs") && !text.contains("big.rs"));
        assert_eq!(char_page("héllo wörld", 3, 4), ("lo w", 7), "offsets count characters");
        assert_eq!(char_page("abc", 5, 4), ("", 5));
    }

    #[tokio::test]
    async fn the_result_lists_every_file_but_shows_only_whole_files_that_fit() {
        let patch = format!("{}{}{}", file_patch("a.rs", "a", &["b"]), big_patch("big.rs", 30_000), file_patch("c.rs", "a", &["b"]));
        let r = rig_on(FakeWorktrees::new().with_patch(patch), &[Claude], TaskType::Feature);
        let res = r.d.delegate(brief("x")).await;
        assert_eq!(res.files_changed(), ["a.rs", "big.rs", "c.rs"], "the stat list is never cut short");
        assert_eq!(res.shown, ["a.rs", "c.rs"]);
        assert!(!res.diff.contains("big.rs") && res.diff.contains("c.rs") && !res.diff.contains("truncated"));
        let text = res.render();
        assert!(text.contains("big.rs +300 -1") && text.contains("NOT SHOWN here") && text.contains(&format!("show_diff({}, file)", res.worker_id)), "{text}");
    }

    #[tokio::test]
    async fn merge_refuses_what_the_planner_has_not_seen_and_files_lets_the_seen_ones_through() {
        let patch = format!("{}{}{}", file_patch("a.rs", "a", &["b"]), big_patch("big.rs", 30_000), file_patch("c.rs", "a", &["b"]));
        let r = rig_on(FakeWorktrees::new().with_patch(patch), &[Claude], TaskType::Feature);
        let id = r.d.delegate(brief("x")).await.worker_id;
        let err = r.d.merge(&id, None).await.unwrap_err();
        assert!(err.contains("big.rs") && err.contains("show_diff") && err.contains("`files`") && err.contains("a.rs, c.rs"), "{err}");
        assert!(!err.contains("not seen in full: a.rs"), "only the unseen file is named: {err}");
        assert!(r.wt.applied.lock().unwrap().is_empty(), "nothing was applied");
        let err = r.d.merge(&id, Some(vec!["big.rs".into()])).await.unwrap_err();
        assert!(err.contains("none of the requested files has been shown"), "{err}");
        assert!(r.d.merge(&id, Some(vec!["nope.rs".into()])).await.unwrap_err().contains("no file `nope.rs`"));
        assert!(r.d.merge(&id, Some(vec![])).await.unwrap_err().contains("`files` is empty"));
        // The seen files go through alone; the rest stays for later.
        let msg = r.d.merge(&id, Some(vec!["a.rs".into(), "c.rs".into(), "a.rs".into()])).await.unwrap();
        assert!(msg.contains("a.rs, c.rs") && msg.contains("Not merged yet") && msg.contains("big.rs"), "{msg}");
        let applied = r.wt.applied.lock().unwrap().clone();
        assert_eq!(applied.len(), 1);
        assert!(applied[0].contains("a/a.rs") && applied[0].contains("a/c.rs") && !applied[0].contains("big.rs"), "only those files' hunks");
        assert!(r.wt.removed.lock().unwrap().is_empty(), "the worker is kept for the rest");
        // Merging the rest later needs it shown first, and then removes the worker.
        assert!(r.d.merge(&id, None).await.unwrap_err().contains("big.rs"));
        let page = r.d.show_diff(&id, Some("big.rs"), 0).await.unwrap();
        assert!(page.contains("characters 0-20000 of 30") && page.contains("[more: call show_diff") && page.contains("offset 20000"), "{}", &page[page.len() - 300..]);
        assert!(r.d.merge(&id, None).await.is_err(), "one page of a two-page file is not the whole file");
        let page = r.d.show_diff(&id, Some("big.rs"), 20_000).await.unwrap();
        assert!(page.contains("[end]") && page.contains("Every file has now been shown in full"));
        let msg = r.d.merge(&id, None).await.unwrap();
        assert!(msg.starts_with(&format!("merged worker {id}: big.rs")), "{msg}");
        assert!(!r.wt.applied.lock().unwrap()[1].contains("a/a.rs"), "files merged before are not applied twice");
        assert_eq!(r.wt.removed.lock().unwrap().len(), 1, "nothing left: the worktree is gone");
    }

    #[tokio::test]
    async fn show_diff_pages_the_whole_diff_and_a_skipped_page_does_not_count_as_seen() {
        let patch = format!("{}{}", big_patch("big.rs", 30_000), file_patch("z.rs", "a", &["b"]));
        let r = rig_on(FakeWorktrees::new().with_patch(patch), &[Claude], TaskType::Feature);
        let id = r.d.delegate(brief("x")).await.worker_id;
        // Jumping to the second page of a file never read from its start shows nothing in full.
        r.d.show_diff(&id, Some("big.rs"), 20_000).await.unwrap();
        assert!(r.d.merge(&id, Some(vec!["big.rs".into()])).await.unwrap_err().contains("not seen in full"));
        // The whole-diff view, read page by page from the start, covers big.rs in full.
        let first = r.d.show_diff(&id, None, 0).await.unwrap();
        assert!(first.contains("characters 0-20000 of 3") && first.contains("Not yet shown in full: big.rs"), "{first}");
        let second = r.d.show_diff(&id, None, 20_000).await.unwrap();
        assert!(second.contains("Every file has now been shown in full"), "{second}");
        assert!(r.d.merge(&id, None).await.unwrap().contains("big.rs, z.rs"));
        // Errors name the problem.
        let id = r.d.delegate(brief("y")).await.worker_id;
        assert!(r.d.show_diff(&id, Some("nope.rs"), 0).await.unwrap_err().contains("changed no file `nope.rs`"));
        assert!(r.d.show_diff(&id, None, 999_999).await.unwrap_err().contains("past the end"));
        assert!(r.d.show_diff("w0-0", None, 0).await.unwrap_err().contains("unknown worker id"));
    }

    // ---------------------------------------------------------------- reserving the slot

    #[tokio::test]
    async fn parallel_delegations_claim_their_slot_before_the_worktree_exists() {
        // Making a worktree takes a while, and all pickers run before any of them finishes.
        let r = rig_on(FakeWorktrees::slow(Duration::from_millis(100)), &[Claude, Codex, Gemini], TaskType::Feature);
        let (a, b, c) = tokio::join!(r.d.delegate(brief("one")), r.d.delegate(brief("two")), r.d.delegate(brief("three")));
        assert!(a.ok && b.ok && c.ok, "{:?} {:?} {:?}", a.error, b.error, c.error);
        let providers: HashSet<_> = [a.provider, b.provider, c.provider].into_iter().collect();
        assert_eq!(providers.len(), 3, "three parallel workers, three providers: {providers:?}");
    }

    #[tokio::test]
    async fn a_reserved_slot_is_released_when_the_worktree_cannot_be_made() {
        let wt = Arc::new(FakeWorktrees { base: temp_dir("nocreate"), create_fails: true, ..FakeWorktrees::default() });
        let r = rig_on(wt, &[Claude], TaskType::Feature);
        let res = r.d.delegate(brief("x")).await;
        assert!(!res.ok && res.error.unwrap().contains("could not create a worktree"));
        assert!(lock(&r.d.live).is_empty(), "no placeholder is left behind to count as a running worker");
    }

    // ---------------------------------------------------------------- git off the async threads

    #[tokio::test]
    async fn slow_git_work_of_parallel_workers_overlaps() {
        let r = rig_on(FakeWorktrees::slow(Duration::from_millis(200)), &[Claude, Codex], TaskType::Feature);
        let started = Instant::now();
        let (a, b) = tokio::join!(r.d.delegate(brief("one")), r.d.delegate(brief("two")));
        assert!(a.ok && b.ok);
        let took = started.elapsed();
        assert!(took < Duration::from_millis(350), "two 200 ms checkouts took {took:?}: they ran one after the other");
        // Merges touch the one main tree, its index and the rollback snapshot: one at a time.
        let started = Instant::now();
        let (x, y) = tokio::join!(r.d.merge(&a.worker_id, None), r.d.merge(&b.worker_id, None));
        assert!(x.is_ok() && y.is_ok());
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(400), "two 200 ms merges took {took:?}: they overlapped on the main tree");
    }

    // ---------------------------------------------------------------- the saved registry

    const HOUR: u64 = 3_600_000;

    /// A fake process table: which fake pids are running. No real pid is ever looked at.
    #[derive(Default)]
    struct Procs {
        alive: Mutex<HashSet<u32>>,
    }

    struct FakeLiveness {
        pid: u32,
        procs: Arc<Procs>,
    }

    impl Liveness for FakeLiveness {
        fn me(&self) -> Owner {
            Owner { pid: self.pid, start: Some(1) }
        }
        fn alive(&self, o: &Owner) -> bool {
            self.procs.alive.lock().unwrap().contains(&o.pid)
        }
    }

    static NEXT_PID: AtomicU64 = AtomicU64::new(1000);

    /// A server on `state` with a fresh fake pid, running in `procs`, and a clock the test moves.
    fn server(wt: &Arc<FakeWorktrees>, state: &Path, clock: &Arc<AtomicU64>, procs: &Arc<Procs>) -> Delegator {
        let started: Started = Arc::new(Mutex::new(Vec::new()));
        let scripts: Scripts = Arc::new(Mutex::new(PROVIDERS.iter().map(|p| (*p, OK)).collect()));
        let c = clock.clone();
        let pid = NEXT_PID.fetch_add(1, Ordering::SeqCst) as u32;
        procs.alive.lock().unwrap().insert(pid);
        delegator_on(wt, &[Claude], TaskType::Feature, &started, &scripts, &Modes::default())
            .with_clock(Box::new(move || c.load(Ordering::SeqCst)))
            .with_liveness(Arc::new(FakeLiveness { pid, procs: procs.clone() }))
            .with_store(state)
    }

    /// A server whose predecessors are all dead (nobody else is in its process table).
    fn saving(wt: &Arc<FakeWorktrees>, state: &Path, clock: &Arc<AtomicU64>) -> Delegator {
        server(wt, state, clock, &Arc::default())
    }

    fn real_now() -> Arc<AtomicU64> {
        Arc::new(AtomicU64::new(log::now_ms()))
    }

    #[tokio::test]
    async fn a_restarted_server_still_merges_and_discards_its_workers() {
        let patch = format!("{}{}", file_patch("a.rs", "a", &["b"]), big_patch("big.rs", 30_000));
        let (wt, state, clock) = (FakeWorktrees::new().with_patch(patch), temp_dir("state"), real_now());
        let first = saving(&wt, &state, &clock);
        let (keep, dropped) = (first.delegate(brief("one")).await.worker_id, first.delegate(brief("two")).await.worker_id);
        first.show_diff(&keep, Some("big.rs"), 0).await.unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(state.join("workers").join(format!("{keep}.json"))).unwrap()).unwrap();
        assert_eq!(saved["worker_id"], keep.as_str());
        assert_eq!(saved["path"], wt.base.join(&keep).to_string_lossy().as_ref());
        assert_eq!(saved["repo"], wt.repo().to_string_lossy().as_ref());
        assert_eq!((saved["provider"].clone(), saved["running"].clone()), (json!("claude"), json!(false)));
        assert!(saved["model"].is_string() && saved["created_ms"].is_u64());
        assert_eq!(saved["shown"], json!(["a.rs"]));
        assert_eq!(saved["covered"], json!({ "big.rs": 20000 }), "page-by-page progress is saved too");
        std::mem::drop(first);
        // The server process ends; a new one starts on the same state directory.
        let second = saving(&wt, &state, &clock);
        second.recover().await;
        assert!(second.merge(&keep, None).await.unwrap_err().contains("big.rs"), "what was shown, and what was not, survived the restart");
        second.show_diff(&keep, Some("big.rs"), 20_000).await.unwrap();
        assert!(second.merge(&keep, None).await.unwrap().contains("merged worker"));
        assert!(second.discard(&dropped).await.unwrap().contains("discarded"));
        assert!(!state.join("workers").join(format!("{keep}.json")).exists() && !state.join("workers").join(format!("{dropped}.json")).exists(), "records go with the worker");
        assert!(second.merge(&keep, None).await.unwrap_err().contains("unknown worker id"));
    }

    #[tokio::test]
    async fn startup_removes_abandoned_checkouts_and_prunes() {
        let (wt, state, clock) = (FakeWorktrees::new(), temp_dir("state"), real_now());
        let first = saving(&wt, &state, &clock);
        let (old, fresh) = (first.delegate(brief("one")).await.worker_id, first.delegate(brief("two")).await.worker_id);
        // `fresh` is a later worker: move the clock before saving it again via a show.
        let mut record: Value = serde_json::from_slice(&std::fs::read(state.join("workers").join(format!("{old}.json"))).unwrap()).unwrap();
        record["created_ms"] = json!(log::now_ms() - 25 * HOUR);
        std::fs::write(state.join("workers").join(format!("{old}.json")), record.to_string()).unwrap();
        // An orphan (no record), a brand new orphan, a corrupt record, and a record of a worker that was running.
        let orphan = wt.base.join("orphan");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(state.join("workers").join("garbage.json"), "{ not json").unwrap();
        std::fs::write(state.join("workers").join("other.txt"), "ignored").unwrap();
        let running = wt.base.join("w1-running");
        std::fs::create_dir_all(&running).unwrap();
        let live = Saved { worker_id: "w1-running".into(), path: running.clone(), repo: wt.repo(), provider: "claude".into(), model: "m".into(), created_ms: log::now_ms(), running: true, shown: Default::default(), covered: Default::default(), merged: Default::default(), owner: None };
        std::fs::write(state.join("workers").join("w1-running.json"), serde_json::to_string(&live).unwrap()).unwrap();
        std::mem::drop(first);

        // Now: the old worker is over 24 h, nothing else is. A brand new orphan is left alone.
        let second = saving(&wt, &state, &clock);
        second.recover().await;
        assert!(wt.removed.lock().unwrap().contains(&wt.base.join(&old)), "older than 24 h: checkout removed");
        assert!(!state.join("workers").join(format!("{old}.json")).exists());
        assert!(wt.base.join(&fresh).exists() && second.merge(&fresh, None).await.is_ok(), "younger workers come back");
        assert!(orphan.exists() && running.exists(), "a fresh orphan and another server's running worker are left alone");
        assert!(state.join("workers").join("garbage.json").exists(), "a corrupt record is skipped, not deleted or fatal");
        assert!(wt.pruned.lock().unwrap().contains(&wt.repo()), "git worktree prune ran in the affected repository");

        // A day later the orphan and the stale running record are cleaned as well.
        let later = Arc::new(AtomicU64::new(log::now_ms() + 25 * HOUR));
        let third = saving(&wt, &state, &later);
        third.recover().await;
        assert!(!orphan.exists(), "an orphan older than the grace period is removed");
        assert!(!running.exists() && !state.join("workers").join("w1-running.json").exists(), "a running record is abandoned after 24 h");
    }

    #[tokio::test]
    async fn a_worker_from_another_repository_is_not_merged_here() {
        let (wt, state, clock) = (FakeWorktrees::new(), temp_dir("state"), real_now());
        let id = saving(&wt, &state, &clock).delegate(brief("one")).await.worker_id;
        let elsewhere = FakeWorktrees::new();
        let other = saving(&elsewhere, &state, &clock);
        other.recover().await;
        let err = other.merge(&id, None).await.unwrap_err();
        assert!(err.contains("was made from") && err.contains("discard"), "{err}");
        assert!(elsewhere.applied.lock().unwrap().is_empty());
        assert!(other.discard(&id).await.is_ok());
    }

    #[test]
    fn leftovers_are_noticed_only_when_there_are_some() {
        let state = temp_dir("state");
        assert!(!has_leftovers(&state));
        std::fs::create_dir_all(state.join("worktrees")).unwrap();
        assert!(!has_leftovers(&state), "an empty directory is nothing");
        std::fs::create_dir_all(state.join("worktrees").join("w1")).unwrap();
        assert!(has_leftovers(&state));
    }

    // ---------------------------------------------------------------- real git: applying on a dirty tree

    /// A repository with `big.txt` of 30 numbered lines, committed.
    fn numbered_repo() -> PathBuf {
        let d = temp_dir("repo");
        crate::testkit::git_repo::init_with_commit(&d, &[("big.txt", &(1..=30).map(|n| format!("line{n}\n")).collect::<String>())]);
        d
    }

    fn replace_line(path: &Path, n: usize, text: &str) {
        let content = std::fs::read_to_string(path).unwrap();
        let out: String = content.lines().enumerate().map(|(i, l)| format!("{}\n", if i + 1 == n { text } else { l })).collect();
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn merge_works_on_top_of_the_planners_unstaged_edit_to_the_same_file() {
        let repo = numbered_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let w = wt.create("w1").unwrap();
        replace_line(&w.join("big.txt"), 3, "from-worker");
        let patch = wt.diff(&w).unwrap();
        // The planner edits another part of the same file and has NOT staged it.
        replace_line(&repo.join("big.txt"), 27, "from-planner");
        wt.apply(&patch).unwrap();
        let merged = std::fs::read_to_string(repo.join("big.txt")).unwrap();
        assert!(merged.contains("from-worker") && merged.contains("from-planner"), "{merged}");
        assert!(!merged.contains("line3\n") && !merged.contains("line27\n") && !merged.contains("<<<<<<<"));
        assert_eq!(run_git(&repo, &["diff", "--cached", "--stat"]).trim(), "", "the planner's index is not touched");
    }

    #[test]
    fn an_overlapping_unstaged_edit_fails_cleanly_and_says_why() {
        let repo = numbered_repo();
        let wt = GitWorktrees::new(&repo, temp_dir("wt-root")).unwrap();
        let w = wt.create("w1").unwrap();
        replace_line(&w.join("big.txt"), 3, "from-worker");
        std::fs::write(w.join("new.txt"), "hello\n").unwrap();
        let patch = wt.diff(&w).unwrap();
        replace_line(&repo.join("big.txt"), 4, "from-planner"); // inside the worker's hunk, unstaged
        let (status, before) = (run_git(&repo, &["status", "--porcelain"]), std::fs::read_to_string(repo.join("big.txt")).unwrap());
        let err = format!("{:#}", wt.apply(&patch).unwrap_err());
        assert!(err.contains("uncommitted edits") && err.contains("nothing was changed"), "{err}");
        assert!(!err.contains("conflict with your tree"), "the cause is the dirty file, not a conflict: {err}");
        assert_eq!(std::fs::read_to_string(repo.join("big.txt")).unwrap(), before, "the file is exactly as it was");
        assert!(!repo.join("new.txt").exists(), "no partial application");
        assert_eq!(run_git(&repo, &["status", "--porcelain"]), status);
    }

    #[test]
    fn a_real_checkout_that_is_gone_is_cleaned_by_remove_and_prune() {
        let repo = temp_repo();
        let root = temp_dir("wt-root");
        let wt = GitWorktrees::new(&repo, root.clone()).unwrap();
        let (gone, kept) = (wt.create("wgone").unwrap(), wt.create("wkept").unwrap());
        std::fs::remove_dir_all(&gone).unwrap();
        assert!(run_git(&repo, &["worktree", "list"]).contains("wgone"), "git still lists the vanished checkout");
        wt.prune(&repo).unwrap();
        let list = run_git(&repo, &["worktree", "list"]);
        assert!(!list.contains("wgone") && list.contains("wkept"), "{list}");
        // A directory git does not know (a leftover of another repository) is deleted, but only under the root.
        let stray = root.join("wstray");
        std::fs::create_dir_all(&stray).unwrap();
        wt.remove(&stray).unwrap();
        assert!(!stray.exists());
        let outside = temp_dir("outside");
        assert!(wt.remove(&outside).is_err() && outside.exists(), "never deletes outside its root");
        wt.remove(&kept).unwrap();
    }

    // ---------------------------------------------------------------- shared state directory: owners

    /// An in-memory [`Records`] with slow saves or loads, to show IO is not done under the
    /// registry lock or on an async thread.
    #[derive(Default)]
    struct FakeRecords {
        save_delay: Duration,
        load_delay: Duration,
        saved: Mutex<HashMap<String, Saved>>,
        saves: AtomicUsize,
    }

    impl Records for FakeRecords {
        fn save(&self, s: &Saved) {
            std::thread::sleep(self.save_delay);
            self.saves.fetch_add(1, Ordering::SeqCst);
            self.saved.lock().unwrap().insert(s.worker_id.clone(), s.clone());
        }
        fn remove(&self, id: &str) {
            self.saved.lock().unwrap().remove(id);
        }
        fn load(&self) -> Vec<Saved> {
            std::thread::sleep(self.load_delay);
            self.saved.lock().unwrap().values().cloned().collect()
        }
    }

    #[test]
    fn a_workers_latest_tool_call_is_kept_beside_its_record_and_goes_with_it() {
        let state = temp_dir("activity");
        let store = Store { dir: state.join("workers") };
        std::fs::create_dir_all(&store.dir).unwrap();
        assert_eq!(store.activity("w1"), None);
        store.note_activity("w1", 3, "Bash   cargo\ntest");
        assert_eq!(store.activity("w1"), Some((3, "Bash cargo test".to_string())));
        store.note_activity("../evil", 1, "x");
        assert!(!state.join("evil.activity").exists(), "only plain ids are written");
        store.remove("w1");
        assert_eq!(store.activity("w1"), None);
    }

    fn record_file(state: &Path, id: &str) -> PathBuf {
        state.join("workers").join(format!("{id}.json"))
    }

    fn read_record(state: &Path, id: &str) -> Value {
        serde_json::from_slice(&std::fs::read(record_file(state, id)).unwrap()).unwrap()
    }

    /// Poll `cond` for up to 3 s: cleanup of a dropped call runs on a plain thread.
    async fn eventually(what: &str, cond: impl Fn() -> bool) {
        for _ in 0..300 {
            if cond() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never became true: {what}");
    }

    #[tokio::test]
    async fn a_second_server_never_touches_the_workers_of_a_live_one() {
        let (wt, state, clock, procs) = (FakeWorktrees::new(), temp_dir("state"), real_now(), Arc::<Procs>::default());
        let first = server(&wt, &state, &clock, &procs);
        let (young, old) = (first.delegate(brief("one")).await.worker_id, first.delegate(brief("two")).await.worker_id);
        assert_eq!(read_record(&state, &young)["owner"]["pid"], first.owner.pid, "every record names its server");
        let mut record = read_record(&state, &old);
        record["created_ms"] = json!(log::now_ms() - 25 * HOUR);
        std::fs::write(record_file(&state, &old), record.to_string()).unwrap();

        // A second session starts while the first is alive and still using both workers.
        let second = server(&wt, &state, &clock, &procs);
        second.recover().await;
        assert!(wt.removed.lock().unwrap().is_empty(), "nothing of a live server's is deleted, however old");
        assert!(record_file(&state, &young).exists() && record_file(&state, &old).exists());
        assert!(second.merge(&young, None).await.unwrap_err().contains("unknown worker id"), "and nothing is adopted");
        assert!(first.lookup(&young).is_ok() && first.lookup(&old).is_ok(), "the first server still has its workers");

        // Once the first server is gone, a third adopts what is recent and removes what is stale.
        procs.alive.lock().unwrap().remove(&first.owner.pid);
        let third = server(&wt, &state, &clock, &procs);
        third.recover().await;
        assert!(wt.removed.lock().unwrap().contains(&wt.base.join(&old)) && !record_file(&state, &old).exists(), "a dead server's stale worker goes");
        assert_eq!(read_record(&state, &young)["owner"]["pid"], third.owner.pid, "an adopted record is restamped");
        assert!(third.merge(&young, None).await.unwrap().contains("merged worker"), "a dead server's recent worker still merges");
    }

    #[test]
    fn what_recovery_does_with_a_record() {
        let saved = |running: bool, age_ms: u64| Saved {
            worker_id: "w1".into(),
            path: PathBuf::from("/x/w1"),
            repo: PathBuf::from("/x"),
            provider: "claude".into(),
            model: "m".into(),
            created_ms: NOW - age_ms,
            running,
            shown: Default::default(),
            covered: Default::default(),
            merged: Default::default(),
            owner: None,
        };
        let fate = |running, age_ms, exists, owner_alive| Found { saved: saved(running, age_ms), exists, owner_alive }.fate(NOW);
        assert_eq!(fate(false, 0, true, true), Fate::Keep, "a live owner's record is in use");
        assert_eq!(fate(false, 48 * HOUR, false, true), Fate::Keep, "even when old and checkout-less");
        assert_eq!(fate(true, 48 * HOUR, true, true), Fate::Keep);
        assert_eq!(fate(false, HOUR, true, false), Fate::Adopt);
        assert_eq!(fate(false, 25 * HOUR, true, false), Fate::Drop);
        assert_eq!(fate(false, HOUR, false, false), Fate::Drop, "no checkout left");
        assert_eq!(fate(true, HOUR, true, false), Fate::Keep, "a worker that was running is not reloaded");
        assert_eq!(fate(true, 25 * HOUR, true, false), Fate::Drop);
        // A record written before owners existed has none, and parses.
        let legacy = r#"{"worker_id":"w1","path":"/x/w1","repo":"/x","provider":"claude","model":"m","created_ms":1}"#;
        assert_eq!(serde_json::from_str::<Saved>(legacy).unwrap().owner, None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_liveness_knows_a_running_process_and_a_recycled_pid() {
        let me = ProcLiveness.me();
        assert!(me.start.is_some(), "the start time is read from /proc");
        assert!(ProcLiveness.alive(&me));
        assert!(ProcLiveness.alive(&Owner { pid: me.pid, start: None }), "pid alone is enough when no start time was saved");
        assert!(!ProcLiveness.alive(&Owner { pid: me.pid, start: me.start.map(|s| s + 1) }), "same pid, other start time: a different process");
        assert!(!ProcLiveness.alive(&Owner { pid: u32::MAX, start: Some(1) }), "no such process");
    }

    // ---------------------------------------------------------------- a dropped delegate call

    fn running_on(d: &Delegator, p: ProviderId) -> usize {
        lock(&d.live).values().filter(|w| w.running && w.provider == p).count()
    }

    #[tokio::test]
    async fn a_delegation_dropped_while_its_worktree_is_made_leaves_nothing_behind() {
        let wt = FakeWorktrees::slow(Duration::from_millis(300));
        let r = rig_on(wt.clone(), &[Claude], TaskType::Feature);
        let out = tokio::time::timeout(Duration::from_millis(100), r.d.delegate(brief("x"))).await;
        assert!(out.is_err(), "the call was dropped mid-flight");
        assert!(lock(&r.d.live).is_empty(), "no placeholder stays behind as a worker that is running forever");
        assert_eq!(running_on(&r.d, Claude), 0, "the provider's live load is back to zero");
        // The checkout that finished after the drop is removed again, not leaked.
        eventually("the late checkout is removed", || {
            let made = wt.created.lock().unwrap().clone();
            made.len() == 1 && wt.removed.lock().unwrap().contains(&made[0]) && !made[0].exists()
        })
        .await;
        assert!(r.d.delegate(brief("y")).await.ok, "and the server delegates normally afterwards");
    }

    #[tokio::test]
    async fn a_delegation_dropped_while_its_worker_runs_removes_worktree_record_and_slot() {
        let records = Arc::new(FakeRecords::default());
        let mut r = rig(&[Claude], TaskType::Feature);
        r.d = r.d.with_records(records.clone()).with_idle_timeout(Duration::from_secs(30));
        r.scripts.lock().unwrap().insert(Claude, "sleep 5");
        let out = tokio::time::timeout(Duration::from_millis(300), r.d.delegate(brief("x"))).await;
        assert!(out.is_err());
        assert!(records.saves.load(Ordering::SeqCst) >= 1, "the worker had been saved before the call was dropped");
        assert!(lock(&r.d.live).is_empty() && running_on(&r.d, Claude) == 0);
        eventually("record and worktree are removed", || records.saved.lock().unwrap().is_empty() && r.wt.removed.lock().unwrap().len() == 1).await;
        assert!(!r.wt.created.lock().unwrap()[0].exists());
    }

    // ---------------------------------------------------------------- no IO under the lock or on async threads

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_slow_record_write_does_not_delay_the_registry() {
        let records = Arc::new(FakeRecords { save_delay: Duration::from_millis(200), ..FakeRecords::default() });
        let mut r = rig(&[Claude, Codex], TaskType::Feature);
        r.d = r.d.with_records(records.clone());
        let d = Arc::new(r.d);
        let spec = models().iter().find(|m| m.provider == Claude).unwrap();
        let mut w = Worker::reserved(spec, d.worktrees.repo(), NOW);
        w.path = Some(d.worktrees.root().join("w1"));
        lock(&d.live).insert("w1".into(), w);
        let writer = {
            let d = d.clone();
            tokio::spawn(async move { d.update("w1", |w| w.running = false).await })
        };
        tokio::time::sleep(Duration::from_millis(40)).await; // the save is under way
        let started = Instant::now();
        d.reserve(&signals(TaskType::Feature), &HashSet::new(), 10, "w2").unwrap();
        d.lookup("w1").unwrap();
        let took = started.elapsed();
        assert!(took < Duration::from_millis(100), "the registry waited {took:?} for a record write");
        writer.await.unwrap();
        assert!(records.saved.lock().unwrap().contains_key("w1"), "the write still happened");
    }

    #[tokio::test]
    async fn recovery_reads_the_disk_off_the_async_threads() {
        let records = Arc::new(FakeRecords { load_delay: Duration::from_millis(200), ..FakeRecords::default() });
        let mut r = rig(&[Claude], TaskType::Feature);
        r.d = r.d.with_records(records);
        let started = Instant::now();
        let tick = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            started.elapsed()
        };
        let ((), tick) = tokio::join!(r.d.recover(), tick);
        assert!(tick < Duration::from_millis(100), "a 10 ms timer fired after {tick:?}: recover blocked the async thread");
    }

    #[tokio::test]
    async fn concurrent_record_writes_end_with_the_latest_state() {
        let records = Arc::new(FakeRecords { save_delay: Duration::from_millis(30), ..FakeRecords::default() });
        let mut r = rig(&[Claude], TaskType::Feature);
        r.d = r.d.with_records(records.clone());
        let spec = models().iter().find(|m| m.provider == Claude).unwrap();
        let mut w = Worker::reserved(spec, r.d.worktrees.repo(), NOW);
        w.path = Some(r.d.worktrees.root().join("w1"));
        lock(&r.d.live).insert("w1".into(), w);
        let (a, b) = tokio::join!(r.d.update("w1", |w| w.merged.insert("a".into())), r.d.update("w1", |w| w.merged.insert("b".into())));
        assert!(a.is_some() && b.is_some());
        let saved = records.saved.lock().unwrap()["w1"].clone();
        assert_eq!(saved.merged.into_iter().collect::<Vec<_>>(), ["a", "b"], "a stale snapshot never overwrites a newer one");
        r.d.take("w1").await.unwrap();
        assert!(records.saved.lock().unwrap().is_empty(), "taking a worker removes its record");
    }

    // ---------------------------------------------------------------- worker ids

    #[test]
    fn servers_started_in_the_same_instant_never_share_worker_ids() {
        let (a, b) = (rig(&[Claude], TaskType::Feature).d, rig(&[Claude], TaskType::Feature).d);
        let ids = |d: &Delegator| (0..50).map(|_| d.next_id()).collect::<HashSet<_>>();
        let (ia, ib) = (ids(&a), ids(&b));
        assert!(ia.is_disjoint(&ib), "two servers made together must not produce the same ids: {:?}", ia.intersection(&ib).collect::<Vec<_>>());
        assert_eq!(ia.len(), 50);
    }

    #[test]
    fn the_run_tag_separates_processes_instants_and_instances_and_is_filename_safe() {
        let base = run_tag(100, 5_000_000_123, 0);
        assert_ne!(base, run_tag(101, 5_000_000_123, 0), "another process in the same nanosecond");
        assert_ne!(base, run_tag(100, 5_000_000_124, 0), "the same process a nanosecond later");
        assert_ne!(base, run_tag(100, 5_000_000_123, 1), "another Delegator in the same process and instant");
        assert_ne!(run_tag(0x1, 0x20, 0), run_tag(0x12, 0x0, 0), "the parts cannot run together");
        assert!(plain_id(&format!("w{}-{}", run_tag(u32::MAX, u128::MAX, u64::MAX), u64::MAX)));
        let usual = format!("w{}-{}", run_tag(4_194_304, 1_760_000_000_123_456_789, 3), 12);
        assert!(usual.len() <= 28, "{usual}");
    }

    // ---------------------------------------------------------------- removing only inside the root

    #[test]
    fn remove_never_deletes_outside_its_root_whatever_the_record_says() {
        let repo = temp_repo();
        let base = temp_dir("jail");
        let root = base.join("root");
        std::fs::create_dir_all(&root).unwrap();
        let victim = base.join("victim");
        std::fs::create_dir_all(victim.join("inner")).unwrap();
        std::fs::write(victim.join("inner").join("keep.txt"), "precious").unwrap();
        let wt = GitWorktrees::new(&repo, root.clone()).unwrap();
        let kept = |why: &str| assert!(victim.join("inner").join("keep.txt").exists(), "{why}");

        // `..` passes a lexical starts_with(root) but leads out of it.
        assert!(wt.remove(&root.join("..").join("victim")).is_err());
        kept("<root>/../victim");
        assert!(wt.remove(&root.join("..").join("..").join(base.file_name().unwrap()).join("victim")).is_err());
        kept("<root>/../../<base>/victim");
        assert!(wt.remove(Path::new("victim")).is_err() && wt.remove(Path::new("root/x")).is_err(), "relative paths are refused");
        // The root itself is never removed.
        assert!(wt.remove(&root).is_err() && wt.remove(&root.join(".")).is_err());
        assert!(root.exists());

        #[cfg(unix)]
        {
            // A symlink inside the root that points out of it: neither through it nor itself.
            std::os::unix::fs::symlink(&victim, root.join("link")).unwrap();
            assert!(wt.remove(&root.join("link").join("inner")).is_err());
            kept("<root>/link/inner");
            assert!(wt.remove(&root.join("link")).is_err());
            kept("<root>/link");
            assert!(root.join("link").exists(), "the link is refused, not followed or removed");
        }

        // A plain leftover under the root still goes, and a vanished one is not an error.
        let stray = root.join("wstray");
        std::fs::create_dir_all(stray.join("deep")).unwrap();
        wt.remove(&stray).unwrap();
        assert!(!stray.exists());
        wt.remove(&root.join("wgone")).unwrap();
        kept("end");
    }

    #[test]
    fn workflows_read_this_projects_open_workers_and_logged_delegations_only() {
        let state = temp_dir("workflows-state");
        let repo = temp_dir("workflows-repo");
        let other = temp_dir("workflows-other");
        let workers = state.join("workers");
        let _ = std::fs::create_dir_all(&workers);
        let record = |id: &str, repo: &Path, running: bool, created: u64| {
            json!({"worker_id": id, "path": repo.join(id), "repo": repo, "provider": "codex", "model": "gpt-6", "created_ms": created, "running": running, "shown": ["a.rs"]}).to_string()
        };
        let _ = std::fs::write(workers.join("w2.json"), record("w2", &repo, false, 20));
        let _ = std::fs::write(workers.join("w1.json"), record("w1", &repo, true, 10));
        let _ = std::fs::write(workers.join("x1.json"), record("x1", &other, true, 5));
        let open = open_workers(&state, &repo);
        assert_eq!(open.iter().map(|w| w.worker_id.as_str()).collect::<Vec<_>>(), vec!["w1", "w2"], "this project's, oldest first");
        assert!(open[0].running && !open[1].running && open[1].shown == 1);

        let key = handoff_key(&repo);
        let row = |id: &str, repo: &str, kind: &str| json!({"ts_ms": 1, "provider": "claude", "model": "m", "ok": true, "wall_ms": 5, "kind": kind, "worker_id": id, "task": "feature", "files_changed": ["a.rs"], "tool_calls": ["Read a.rs", "Edit a.rs"], "repo": repo}).to_string();
        let log = state.join("orchestration.jsonl");
        let lines = [row("w0", &key, "delegate"), "not json".into(), row("x0", "/elsewhere", "delegate"), row("t0", &key, "turn"), row("w1", &key, "delegate")];
        let _ = std::fs::write(&log, lines.join("\n"));
        let recent = recent_delegations(&log, &repo, 10);
        assert_eq!(recent.iter().map(|d| d.worker_id.as_deref().unwrap_or("")).collect::<Vec<_>>(), vec!["w0", "w1"]);
        assert_eq!((recent[1].files_changed.len(), recent[1].tool_calls.len()), (1, 2));
        assert_eq!(recent_delegations(&log, &repo, 1).len(), 1, "the limit keeps the newest");
        assert!(recent_delegations(&state.join("missing.jsonl"), &repo, 10).is_empty());
    }

    #[test]
    fn golden_delegate_log_row() {
        let log_path = temp_dir("golden-log").join("orchestration.jsonl");
        let r = rig(&[Claude], TaskType::Feature);
        let d = r.d.with_log(Some(log_path.to_string_lossy().into_owned()));
        let spec = |p, id| crate::catalog::domain::model_table::find(p, id).unwrap();
        let ok = Ok(TurnOutcome {
            text: "did the piece".into(),
            session_id: None,
            usage: Usage { input_tokens: 10, cached_input_tokens: 4, output_tokens: 7, context_tokens: Some(9) },
            plan: None,
        });
        let quota = Err(TurnError { kind: ErrorKind::Quota { resets_at: Some("5pm".into()) }, message: "limit \"hit\"\n\té ✓ \\".into() });
        let jev = json!({ "task": "feature", "edit_p": 0.5 });
        let files = ["src/net.rs".to_string()];
        let tools = ["Edit src/net.rs".to_string()];
        let typical = AttemptLog {
            spec: spec(Claude, "claude-sonnet-5-5"), id: "w1", task: TaskType::Feature, attempt: 1, read_only: false, jev: &jev, result: &ok,
            wall_ms: 1500, started_ms: 1_760_000_000_000, files: &files, tools: &tools, landing: Some("review_no_tests"), in_place: false,
        };
        let odd_files = ["dir with space/é.rs".to_string(), "q\"uote.rs".to_string()];
        let odd_tools = ["Bash echo \"hi\"\tthere".to_string()];
        let odd = AttemptLog {
            spec: spec(Claude, "claude-opus-5-5"), id: "w-2_x", task: TaskType::HardDebug, attempt: 3, read_only: true, jev: &Value::Null, result: &quota,
            wall_ms: 0, started_ms: u64::MAX, files: &odd_files, tools: &odd_tools, landing: None, in_place: true,
        };
        d.log_attempt(&typical);
        d.log_attempt(&odd);
        let repo = handoff_key(&r.wt.repo());
        let text = std::fs::read_to_string(&log_path).unwrap();
        let mut lines = text.lines().map(|l| {
            let mut v: Value = serde_json::from_str(l).unwrap();
            v["ts_ms"] = json!("<TS>");
            assert_eq!(v["repo"], json!(repo));
            v["repo"] = json!("<REPO>");
            format!("{v}\n")
        });
        let actual = format!("## typical: one successful attempt\n{}## unusual: failed read-only attempt, escapes, maximum start time\n{}", lines.next().unwrap(), lines.next().unwrap());
        let golden = golden!("orchestration/delegate_log_row.txt");
        assert_eq!(golden.split_once('\n').unwrap().1, actual);
    }

    #[test]
    fn golden_worker_record() {
        let dir = temp_dir("golden-workers");
        let store = Store { dir: dir.join("workers") };
        let typical = Saved {
            worker_id: "w1".into(),
            path: "/work/wt/w1".into(),
            repo: "/work/app".into(),
            provider: "claude".into(),
            model: "claude-sonnet-5-5".into(),
            created_ms: 1_760_000_000_000,
            running: true,
            shown: Default::default(),
            covered: Default::default(),
            merged: Default::default(),
            owner: Some(Owner { pid: 4242, start: Some(1_759_999_000_000) }),
        };
        let odd = Saved {
            worker_id: "w-2_x".into(),
            path: "/work/wt/with space/é \"q\"".into(),
            repo: "/work/app".into(),
            provider: "codex".into(),
            model: String::new(),
            created_ms: 0,
            running: false,
            shown: ["src/b.rs".to_string(), "src/a.rs".to_string()].into_iter().collect(),
            covered: [("src/b.rs".to_string(), 2), ("src/a.rs".to_string(), 0)].into_iter().collect(),
            merged: ["src/a.rs".to_string()].into_iter().collect(),
            owner: None,
        };
        let mut actual = String::new();
        for (case, saved) in [("typical: a running worker with an owner", &typical), ("unusual: finished, partly merged, no owner, odd path", &odd)] {
            store.save(saved);
            let bytes = std::fs::read_to_string(store.file(&saved.worker_id)).unwrap();
            actual.push_str(&format!("## {case} (file {}.json, no trailing newline)\n{bytes}\n", saved.worker_id));
        }
        let golden = golden!("orchestration/worker_record.txt");
        assert_eq!(golden.split_once('\n').unwrap().1, actual);
    }

}
