use super::*;

#[tokio::test]
async fn a_stalled_jev_connection_keeps_the_current_model_instead_of_hanging() {
    // Accepts the connection, then never answers.
    let url = stalled_jev().await;
    let router = Router::new(TypeSafe::with_url("test-key", &url), 1.0)
        .with_jev_timeout(std::time::Duration::from_millis(200))
        .with_top_threshold(Some(0.9));
    let started = std::time::Instant::now();
    let w = router.code_want("why does the upload hang on large files", "", 2).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
    assert_eq!(w.tier, 2, "keeps the fallback (current) tier");
    assert!(!w.confident);
    assert!(w.reason.starts_with("jev unavailable (timed out"), "{}", w.reason);
    assert!(w.signals.is_null());
}
