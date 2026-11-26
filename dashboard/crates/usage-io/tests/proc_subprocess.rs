//! Subprocess and login probe tests using only local stubs.
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

use jev_usage_core::provider::ProviderKey;
use jev_usage_io::probe::{CliProbe, LoginProbe, Readiness};
use jev_usage_io::proc::{resolve_in, Flow, ProcRunner, ProcSpec, Program, Resolved, Stderr};
use std::{os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc, time::Duration};

fn stub() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/stubs/status.sh")
}

#[tokio::test]
async fn status_probe_and_sanitized_child_environment() {
    let dir = tempfile::tempdir().unwrap();
    let runner = Arc::new(ProcRunner::with_programs(
        dir.path().to_owned(),
        stub(),
        stub(),
    ));
    let facts = CliProbe::new(Arc::clone(&runner))
        .probe(vec![ProviderKey::Codex, ProviderKey::Claude])
        .await;
    assert_eq!(facts.readiness(ProviderKey::Codex), Readiness::Ready);
    assert_eq!(facts.readiness(ProviderKey::Claude), Readiness::Ready);
    assert_eq!(facts.plan(ProviderKey::Claude).unwrap().as_str(), "max_5x");
    let spec = ProcSpec {
        program: Program::Codex,
        args: &[],
        stdin: b"",
        deadline: Duration::from_secs(5),
        stderr: Stderr::Discard,
        max_output: 1024,
    };
    let mut output = Vec::new();
    assert!(
        runner
            .run(&spec, &mut |line| {
                output.extend_from_slice(line);
                Flow::Continue
            })
            .await
            .unwrap()
            .success
    );
    assert_eq!(
        String::from_utf8(output).unwrap().trim(),
        format!("unset|1|dumb|{}", dir.path().display())
    );
}

#[test]
fn launcher_requires_an_executable_installed_version() {
    let dir = tempfile::tempdir().unwrap();
    let wrappers = dir.path().join("command-wrappers/bin");
    std::fs::create_dir_all(&wrappers).unwrap();
    let wrapper = wrappers.join("codex");
    std::fs::write(&wrapper, "stub").unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let data = dir.path().join("mise");
    let dirs = vec![wrappers];
    assert_eq!(
        resolve_in("codex", &dirs, Some(&data)),
        Resolved::NotInstalled
    );
    let version = data.join("installs/codex/v1/bin");
    std::fs::create_dir_all(&version).unwrap();
    let executable = version.join("codex");
    std::fs::write(&executable, "stub").unwrap();
    assert_eq!(
        resolve_in("codex", &dirs, Some(&data)),
        Resolved::NotInstalled
    );
    std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(resolve_in("codex", &dirs, Some(&data)), Resolved::Installed);
}
