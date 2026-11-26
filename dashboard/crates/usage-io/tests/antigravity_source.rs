//! The Antigravity source against the stub `agy`; no real CLI is ever run.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use jev_usage_core::port::{SourceError, UsageSource};
use jev_usage_core::usage::Unavailable;
use jev_usage_io::clock::SystemClock;
use jev_usage_io::proc::ProcRunner;
use jev_usage_io::sources::antigravity::AntigravityUsageSource;
use std::path::PathBuf;
use std::sync::Arc;

fn source(runner: ProcRunner) -> AntigravityUsageSource {
    AntigravityUsageSource::new(Arc::new(runner), Arc::new(SystemClock))
}

#[tokio::test]
async fn reads_the_stub_with_the_updater_switched_off() {
    // The stub answers only when `AGY_CLI_DISABLE_AUTO_UPDATE=true` reaches it.
    let dir = tempfile::tempdir().unwrap();
    let stub = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../e2e/smoke/stubs/agy");
    let runner = ProcRunner::with_programs(
        dir.path().to_owned(),
        dir.path().join("codex"),
        dir.path().join("claude"),
    )
    .with_agy(stub.canonicalize().unwrap());
    let reading = source(runner).fetch().await.unwrap();
    assert_eq!(reading.meters.as_slice().len(), 4);
    assert!(!format!("{reading:?}").contains("JEVCANARY"));
}

#[tokio::test]
async fn a_test_runner_never_reaches_the_real_agy() {
    let dir = tempfile::tempdir().unwrap();
    let runner = ProcRunner::with_programs(
        dir.path().to_owned(),
        dir.path().join("codex"),
        dir.path().join("claude"),
    );
    assert_eq!(
        source(runner).fetch().await.unwrap_err(),
        SourceError::Unavailable(Unavailable::NotInstalled)
    );
}
