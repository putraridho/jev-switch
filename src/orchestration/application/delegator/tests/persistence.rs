use super::*;

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

/// An in-memory [`Records`] with slow saves or loads, to show IO is not done under the
/// registry lock or on an async thread.
#[derive(Default)]
pub(super) struct FakeRecords {
    pub(super) save_delay: Duration,
    pub(super) load_delay: Duration,
    pub(super) saved: Mutex<HashMap<String, Saved>>,
    pub(super) saves: AtomicUsize,
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
pub(super) async fn eventually(what: &str, cond: impl Fn() -> bool) {
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

pub(super) fn running_on(d: &Delegator, p: ProviderId) -> usize {
    lock(&d.live).values().filter(|w| w.running && w.provider == p).count()
}
