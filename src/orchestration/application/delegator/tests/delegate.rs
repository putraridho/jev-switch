use super::*;

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
    let d = Delegator::new(Runtime::for_tests(avail), judge, Box::new(wt.clone()), real_adapters())
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
