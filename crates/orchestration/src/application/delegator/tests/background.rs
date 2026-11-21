use super::*;

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
