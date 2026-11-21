use super::persistence::{eventually, running_on, FakeRecords};
use super::*;

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
    d.reserve(&BriefSignals::editing(TaskType::Feature), &HashSet::new(), 10, "w2").unwrap();
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
