//! The start-up performance budget, checked against the real `code` binary with no provider
//! CLI on `PATH` (so nothing real runs and no quota is spent). The second start must read
//! the probe cache and stay within every phase budget of [`jev_router::perf::Phase::budget`].

use std::path::Path;
use std::process::{Command, Output};

fn profile(root: &Path, extra_env: &[(&str, &str)]) -> Output {
    let (home, state, bin, project) = (root.join("home"), root.join("state"), root.join("bin"), root.join("project"));
    for d in [&home, &state, &bin, &project] {
        assert!(std::fs::create_dir_all(d).is_ok());
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_code"));
    cmd.arg("--startup-profile")
        .arg("--no-color")
        .arg(&project)
        .env_clear()
        .env("HOME", &home)
        .env("XDG_STATE_HOME", &state)
        .env("PATH", &bin)
        .env("JEV_API_KEY", "test-key-not-real");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.output().unwrap_or_else(|e| panic!("cannot run the code binary: {e}"))
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn temp_root(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("jev-perf-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

#[test]
fn the_second_start_reads_the_probe_cache_and_stays_within_budget() {
    let root = temp_root("budget");
    let first = profile(&root, &[]);
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    assert!(stdout(&first).contains("providers live"), "{}", stdout(&first));

    let second = profile(&root, &[]);
    let report = stdout(&second);
    assert!(second.status.success(), "{}", String::from_utf8_lossy(&second.stderr));
    assert!(report.contains("providers cached"), "{report}");
    for phase in ["jev client", "provider probes", "saved state", "engine", "banner"] {
        assert!(report.contains(phase), "{report}");
    }
    assert!(report.contains("every phase is within its budget"), "start-up over budget:\n{report}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_cache_can_be_turned_off_and_the_start_is_live_again() {
    let root = temp_root("off");
    let _ = profile(&root, &[]);
    let out = profile(&root, &[("JEV_PROBE_CACHE", "off")]);
    assert!(stdout(&out).contains("providers live"), "{}", stdout(&out));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_changed_path_does_not_reuse_the_cache() {
    let root = temp_root("path");
    let _ = profile(&root, &[]);
    let other = root.join("bin2");
    assert!(std::fs::create_dir_all(&other).is_ok());
    let out = profile(&root, &[("PATH", other.to_str().unwrap_or(""))]);
    assert!(stdout(&out).contains("providers live"), "{}", stdout(&out));
    let _ = std::fs::remove_dir_all(&root);
}
