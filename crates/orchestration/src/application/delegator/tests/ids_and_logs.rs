use super::*;

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

#[test]
fn remove_never_deletes_outside_its_root_whatever_the_record_says() {
    let repo = temp_dir("repo");
    jev_testkit::git_repo::init_with_commit(&repo, &[("f.txt", "a\nb\nc\nd\ne\n")]);
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
    let spec = |p, id| jev_catalog::domain::model_table::find(p, id).unwrap();
    let ok = Ok(TurnOutcome {
        text: "did the piece".into(),
        session_id: None,
        usage: Usage { input_tokens: 10, cached_input_tokens: 4, output_tokens: 7, context_tokens: Some(9), ..Usage::default() },
        plan: None,
    });
    let quota = Err(TurnError { kind: ErrorKind::Quota { resets_at: Some("5pm".into()) }, message: "limit \"hit\"\n\té ✓ \\".into(), usage: None });
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
