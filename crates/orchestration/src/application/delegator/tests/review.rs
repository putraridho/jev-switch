use super::*;

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
