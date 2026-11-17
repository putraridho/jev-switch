//! The worker side of cross-provider orchestration: a planner model calls a
//! `delegate` tool with a brief, and a cheaper WORKER model, possibly from another
//! provider, does that piece in its own git worktree. The planner then reviews the
//! returned diff and decides what to merge.
//!
//! **Jev makes every model decision.** Nothing here takes a model from the caller: the
//! tool has no model parameter and [`DelegateRequest`] has no model field, so anything a
//! planner says about models cannot reach this code. Code only
//! 1. filters to the models that are really usable (logged in, not cooling down, billing
//!    allowed: [`crate::providers::Runtime`], [`crate::catalog::Usable`]),
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
//! **The planner merges only what it saw.** The delegate result lists every changed file
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
//! servers never produce the same id. A `delegate` call that is dropped mid-flight removes
//! its placeholder, record and worktree ([`Reservation`]). A path read from a record is
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
use crate::catalog::{
    allowed_for, chain_with, models, pick, Allowances, ModelSpec, Overrides, TaskType, Tier, Usable,
};
use crate::log;
use crate::providers::Runtime;
use crate::router::{ProviderWant, Router, CODE_EDIT_P, CODE_SECURITY_P};
use crate::turn::{idle_timeout_from_env, BackendFactory};
use crate::turnlog::turn_row;

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
    if n >= 3 && (n - 3) % 2 == 0 {
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
pub fn worker_prompt(brief: &str, files: &[String]) -> String {
    let hints = if files.is_empty() {
        String::new()
    } else {
        format!("\n\nFiles that are probably involved (hints, not limits): {}", files.join(", "))
    };
    format!(
        "You are a worker on one piece of a larger task. Rules: work only inside the current directory (your own git \
         worktree) and do not read or write outside it; do not use the network; do not commit or touch git branches. \
         When you are done, reply with a short report: what you changed, and anything you could not do.\n\n\
         Task:\n{brief}{hints}"
    )
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
        git(path, &["diff", "--cached", "--binary", "HEAD"], None)
    }

    /// A plain `git apply` first: it works on the working tree whatever is unstaged, as
    /// long as the hunks fit. Only when it does not fit, `--3way`, which also merges into
    /// text the planner changed but needs the index to match the files.
    fn apply(&self, patch: &str) -> Result<()> {
        let snapshot = Snapshot::take(&self.repo, self.index_path(), &patch_paths(patch));
        let plain = git(&self.repo, &["apply", "--check", "-"], Some(patch)).and_then(|_| git(&self.repo, &["apply", "-"], Some(patch)));
        let Err(plain) = plain else { return Ok(()) };
        match git(&self.repo, &["apply", "--3way", "-"], Some(patch)) {
            Ok(_) => Ok(()),
            Err(three_way) => {
                snapshot.restore();
                Err(anyhow!("{}", explain_apply_failure(&format!("{plain:#}."), &format!("{three_way:#}"))))
            }
        }
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
        let mut out = vec![format!("worker {} ({who}): {}", self.worker_id, if self.ok { "done" } else { "FAILED" })];
        out.extend(self.retries.iter().map(|r| format!("retry: {r}")));
        if let Some(e) = &self.error {
            out.push(format!("error: {e}"));
        }
        if self.ok {
            let id = &self.worker_id;
            if self.stats.is_empty() {
                out.push("files changed: none".into());
            } else {
                out.push(format!("files changed ({}):", self.stats.len()));
                out.extend(self.stats.iter().map(|s| format!("  {}", s.render())));
            }
            out.push(format!("worker report:\n{}", self.summary));
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
        }
        out.join("\n")
    }
}

/// What the MCP server calls. Implemented by [`Delegator`]; tests use a fake.
pub trait DelegateApi: Send + Sync {
    fn delegate(&self, req: DelegateRequest) -> BoxFut<'_, DelegateResult>;
    /// One page of a worker's diff, a single file's when `file` is given, from character
    /// `offset`. What it shows in full counts as seen for `merge`.
    fn show_diff(&self, worker_id: String, file: Option<String>, offset: usize) -> BoxFut<'_, Result<String, String>>;
    /// Apply the worker's diff (only `files` when given) to the main tree, refusing any
    /// file not shown in full; remove its worktree once nothing is left.
    fn merge(&self, worker_id: String, files: Option<Vec<String>>) -> BoxFut<'_, Result<String, String>>;
    /// Remove the worker's worktree without applying anything.
    fn discard(&self, worker_id: String) -> BoxFut<'_, Result<String, String>>;
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
            backend_factory: Box::new(crate::providers::backend_for),
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
        }
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
    async fn run_worker(&self, spec: &'static ModelSpec, dir: &Path, prompt: String) -> (Result<TurnOutcome, TurnError>, Vec<String>) {
        let backend = (self.backend_factory)(spec.provider);
        let req = TurnRequest { dir: dir.to_path_buf(), model: spec.id.to_string(), prompt, session_id: None, mode: Default::default() };
        let mut tools: Vec<String> = Vec::new();
        let mut on_event = |e: &Event| match e {
            Event::Tool(summary) => tools.push(summary.clone()),
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
            o.insert("read_only".into(), json!(a.read_only));
            o.insert("files_changed".into(), json!(a.files));
            o.insert("tool_calls".into(), json!(a.tools));
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
    ) -> Result<DelegateResult, AttemptOutcome> {
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
    ) -> Result<DelegateResult, AttemptOutcome> {
        let made = self.blocking({
            let (id, slot) = (id.clone(), reservation.slot());
            move |w| create_unless_cancelled(w, &id, &slot)
        });
        let path = match made.await {
            Ok(path) => path,
            Err(e) => {
                self.take(&id).await;
                return Err(AttemptOutcome::Stop(format!("could not create a worktree: {e:#}")));
            }
        };
        self.update(&id, |w| w.path = Some(path.clone())).await;
        let started = Instant::now();
        let (result, tools) = self.run_worker(spec, &path, worker_prompt(&req.brief, &req.files)).await;
        self.update(&id, |w| w.running = false).await;
        let wall_ms = started.elapsed().as_millis() as u64;
        let task = worker_task(signals);
        let log = |result: &Result<TurnOutcome, TurnError>, files: &[String]| {
            self.log_attempt(&AttemptLog { spec, id: &id, task, attempt, read_only: req.read_only, jev, result, wall_ms, files, tools: &tools })
        };
        match result {
            Err(err) => {
                log(&Err(err.clone()), &[]);
                // A failed worker's partial work is never offered; its worktree goes.
                let _ = self.forget(&id).await;
                Err(AttemptOutcome::Failed(AttemptFailure { id, spec, err }))
            }
            Ok(out) => {
                let read = self.blocking({
                    let path = path.clone();
                    move |w| w.diff(&path)
                });
                match read.await {
                    Err(e) => {
                        log(&Ok(out), &[]);
                        let _ = self.forget(&id).await;
                        Err(AttemptOutcome::Stop(format!("worker {id} finished but its changes could not be read: {e:#}")))
                    }
                    Ok(diff) => {
                        let parts = split_patch(&diff);
                        let stats: Vec<FileStat> = parts.iter().map(FileDiff::stat).collect();
                        let (shown_text, shown) = first_page(&parts, MAX_DIFF_CHARS);
                        self.update(&id, |w| w.shown.extend(shown.iter().cloned())).await;
                        let files: Vec<String> = stats.iter().map(|s| s.path.clone()).collect();
                        log(&Ok(out.clone()), &files);
                        Ok(DelegateResult {
                            worker_id: id,
                            provider: Some(spec.provider),
                            model: Some(spec.id.to_string()),
                            summary: truncate_with_note(out.text.trim(), MAX_SUMMARY_CHARS, "report"),
                            stats,
                            diff: shown_text,
                            shown,
                            ok: true,
                            error: None,
                            retries: vec![],
                        })
                    }
                }
            }
        }
    }

    /// Judge the brief, then run workers until one succeeds or the attempts are used up.
    pub async fn delegate(&self, req: DelegateRequest) -> DelegateResult {
        let want = (self.judge)(req.brief.clone()).await;
        let signals = want.as_ref().map_or_else(BriefSignals::unjudged, |w| BriefSignals::from_want(w, req.read_only));
        let jev = want.map_or(Value::Null, |w| w.signals);
        let mut excluded: HashSet<(ProviderId, String)> = HashSet::new();
        let mut retries: Vec<String> = Vec::new();
        for attempt in 1..=MAX_WORKER_ATTEMPTS {
            match self.attempt(&req, &signals, &jev, &excluded, attempt).await {
                Ok(mut done) => {
                    done.retries = retries;
                    return done;
                }
                Err(AttemptOutcome::Stop(message)) => return DelegateResult::failed(message, retries),
                Err(AttemptOutcome::Failed(f)) => {
                    let react = lock(&self.runtime).react(f.spec.provider, f.spec.id, &f.err, &mut excluded, (self.now)());
                    match react {
                        Some(note) if attempt < MAX_WORKER_ATTEMPTS => retries.push(note),
                        _ => {
                            let mut failed = DelegateResult::failed(format!("{} {} failed: {}", f.spec.provider, f.spec.id, f.err.message), retries);
                            failed.worker_id = f.id;
                            failed.provider = Some(f.spec.provider);
                            failed.model = Some(f.spec.id.to_string());
                            return failed;
                        }
                    }
                }
            }
        }
        DelegateResult::failed("no worker succeeded".into(), retries)
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
        self.blocking(move |w| w.apply(&patch)).await.map_err(|e| format!("{e:#}\nWorker {id}'s worktree is kept; discard({id}) it or merge by hand."))?;
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
}

impl DelegateApi for Delegator {
    fn delegate(&self, req: DelegateRequest) -> BoxFut<'_, DelegateResult> {
        Box::pin(Delegator::delegate(self, req))
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
    files: &'a [String],
    tools: &'a [String],
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use tokio::process::Command as TokioCommand;

    use crate::availability::{Availability, State};
    use crate::backend::{classify_common, Backend, ErrorKind, Finish, Usage};
    use crate::router::Security;
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
        let p = worker_prompt("add a retry", &["src/net.rs".into()]);
        assert!(p.contains("only inside the current directory") && p.contains("do not use the network"));
        assert!(p.contains("short report") && p.contains("add a retry") && p.contains("src/net.rs"));
        assert!(!worker_prompt("x", &[]).contains("Files that are probably involved"));
    }

    // ---------------------------------------------------------------- the Delegator

    /// A "CLI" that is a shell script with canned lines: `FIN:text`, `FAIL:message`.
    struct Fake {
        provider: ProviderId,
        script: &'static str,
        started: Arc<Mutex<Vec<(ProviderId, String, PathBuf, String)>>>,
    }

    impl Backend for Fake {
        fn provider(&self) -> ProviderId {
            self.provider
        }
        fn command(&self, req: &TurnRequest) -> TokioCommand {
            self.started.lock().unwrap().push((self.provider, req.model.clone(), req.dir.clone(), req.prompt.clone()));
            let mut c = TokioCommand::new("sh");
            c.arg("-c").arg(self.script).current_dir(&req.dir);
            c
        }
        fn parse_line(&self, line: &str) -> Vec<Event> {
            let line = line.trim_end();
            match line.split_once(':') {
                Some(("FIN", t)) => vec![Event::Finished(Finish {
                    text: t.into(),
                    session_id: None,
                    usage: Usage { output_tokens: 7, ..Usage::default() },
                })],
                Some(("FAIL", m)) => vec![Event::Failed { message: m.into() }],
                Some(("TOOL", t)) => vec![Event::Tool(t.into())],
                _ => vec![],
            }
        }
        fn classify_error(&self, message: &str) -> ErrorKind {
            classify_common(message).unwrap_or(ErrorKind::Other)
        }
    }

    const OK: &str = "printf 'TOOL:Edit a.rs\\nFIN:did the piece\\n'";
    const QUOTA: &str = "printf 'FAIL:usage limit reached|1794853451\\n'";

    /// Directories made and removed, and what `apply` was given; no git involved.
    #[derive(Default)]
    struct FakeWorktrees {
        base: PathBuf,
        created: Mutex<Vec<PathBuf>>,
        removed: Mutex<Vec<PathBuf>>,
        applied: Mutex<Vec<String>>,
        pruned: Mutex<Vec<PathBuf>>,
        apply_fails: bool,
        create_fails: bool,
        /// Every `create` and `apply` takes this long, like a big checkout would.
        delay: Duration,
        /// What `diff` returns for every worker; by default a one-file patch named after it.
        patch: Mutex<Option<String>>,
    }

    static UNIQUE: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let n = UNIQUE.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("jev-delegate-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    impl FakeWorktrees {
        fn new() -> Arc<Self> {
            Arc::new(Self { base: temp_dir("fake"), ..Self::default() })
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
            orchestration: crate::router::Orchestration::default(),
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
    }

    /// A delegator on `wt` whose fake CLIs all succeed unless `scripts` says otherwise.
    fn delegator_on(wt: &Arc<FakeWorktrees>, ready: &[ProviderId], task: TaskType, started: &Started, scripts: &Scripts) -> Delegator {
        let avail = PROVIDERS.iter().map(|p| availability(*p, ready.contains(p))).collect();
        let (st, sc) = (started.clone(), scripts.clone());
        let judge: Judge = Box::new(move |_| Box::pin(async move { Some(want(task)) }));
        Delegator::new(Runtime::for_tests(avail), judge, Box::new(wt.clone()))
            .with_backend_factory(Box::new(move |p| {
                let script = sc.lock().unwrap()[&p];
                Box::new(Fake { provider: p, script, started: st.clone() })
            }))
            .with_clock(Box::new(|| NOW))
            .with_idle_timeout(Duration::from_millis(500))
            .with_planner("claude:claude-opus-5-5".into())
    }

    fn rig_on(wt: Arc<FakeWorktrees>, ready: &[ProviderId], task: TaskType) -> Rig {
        let started: Started = Arc::new(Mutex::new(Vec::new()));
        let scripts: Scripts = Arc::new(Mutex::new(PROVIDERS.iter().map(|p| (*p, OK)).collect()));
        let d = delegator_on(&wt, ready, task, &started, &scripts);
        Rig { d, wt, started, scripts }
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
            .with_backend_factory(Box::new(move |p| Box::new(Fake { provider: p, script: OK, started: st.clone() })))
            .with_idle_timeout(Duration::from_millis(500));
        let res = d.delegate(brief("x")).await;
        let err = d.merge(&res.worker_id, None).await.unwrap_err();
        assert!(err.contains("conflict") && err.contains("discard"), "{err}");
        assert!(wt.removed.lock().unwrap().is_empty());
        assert!(d.discard(&res.worker_id).await.unwrap().contains("discarded"));
        assert_eq!(wt.removed.lock().unwrap().len(), 1);
        assert!(d.discard(&res.worker_id).await.is_err());
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
    }

    // ---------------------------------------------------------------- real git

    fn run_git(dir: &Path, args: &[&str]) -> String {
        git(dir, args, None).unwrap_or_else(|e| panic!("{e:#}"))
    }

    /// A repository with one commit holding `f.txt` (five lines).
    fn temp_repo() -> PathBuf {
        let d = temp_dir("repo");
        run_git(&d, &["init", "-q"]);
        std::fs::write(d.join("f.txt"), "a\nb\nc\nd\ne\n").unwrap();
        run_git(&d, &["add", "."]);
        run_git(&d, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);
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
        let started = Instant::now();
        let (x, y) = tokio::join!(r.d.merge(&a.worker_id, None), r.d.merge(&b.worker_id, None));
        assert!(x.is_ok() && y.is_ok());
        let took = started.elapsed();
        assert!(took < Duration::from_millis(350), "two 200 ms merges took {took:?}");
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
        delegator_on(wt, &[Claude], TaskType::Feature, &started, &scripts)
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
        run_git(&d, &["init", "-q"]);
        std::fs::write(d.join("big.txt"), (1..=30).map(|n| format!("line{n}\n")).collect::<String>()).unwrap();
        run_git(&d, &["add", "."]);
        run_git(&d, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);
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
}
