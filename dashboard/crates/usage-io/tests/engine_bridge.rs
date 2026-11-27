//! The engine port contract (DESIGN 5.1): the behaviour Flashboard relies on, pinned so a refactor
//! of the engine that changes it fails here and not in a user's tray.
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
use jev_usage_io::engine_bridge;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---- pure rules -------------------------------------------------------------------------

#[test]
fn env_file_text_cases() {
    let text = "A=1\nexport B = \"two\"\n  C='three'  \n# D=commented\nE=x=y\nF=\nF=later\n";
    assert_eq!(engine_bridge::parse_env(text, "A").as_deref(), Some("1"));
    assert_eq!(
        engine_bridge::parse_env(text, "B").as_deref(),
        Some("two"),
        "export and double quotes"
    );
    assert_eq!(
        engine_bridge::parse_env(text, "C").as_deref(),
        Some("three"),
        "single quotes and padding"
    );
    assert_eq!(
        engine_bridge::parse_env(text, "D"),
        None,
        "a commented line is not a key"
    );
    assert_eq!(
        engine_bridge::parse_env(text, "E").as_deref(),
        Some("x=y"),
        "inner equals signs are kept"
    );
    assert_eq!(
        engine_bridge::parse_env(text, "F").as_deref(),
        Some("later"),
        "an empty value is skipped"
    );
    assert_eq!(engine_bridge::parse_env(text, "MISSING"), None);
    assert_eq!(engine_bridge::parse_env("", "A"), None);
    assert_eq!(
        engine_bridge::parse_env("A=\nB=\"\"\n", "B"),
        None,
        "quotes around nothing are empty"
    );
    assert_eq!(
        engine_bridge::parse_env("MY_A=1\nA_B=2\n", "A"),
        None,
        "the name must match exactly"
    );
    assert_eq!(engine_bridge::parse_env("no equals sign\n", "no"), None);
}

#[test]
fn codex_login_truth_table() {
    let cases = [
        (true, "Logged in using ChatGPT", Some(true)),
        (true, "logged in using an API key", Some(true)),
        (true, "Not logged in", Some(false)),
        (false, "Not logged in", Some(false)),
        (false, "Logged in using ChatGPT", Some(false)),
        (false, "", Some(false)),
        (false, "garbage", Some(false)),
        (true, "", None),
        (true, "garbage", None),
        (true, "NOT LOGGED IN", Some(false)),
        (
            true,
            "warning: something\nLogged in using ChatGPT\n",
            Some(true),
        ),
    ];
    for (exit_ok, text, expected) in cases {
        assert_eq!(
            engine_bridge::codex_logged_in(exit_ok, text),
            expected,
            "exit_ok={exit_ok} text={text:?}"
        );
    }
}

#[test]
fn the_two_login_commands_and_none_elsewhere() {
    assert_eq!(
        engine_bridge::login_command(ProviderKey::Codex),
        Some("codex login")
    );
    assert_eq!(
        engine_bridge::login_command(ProviderKey::Claude),
        Some("claude auth login")
    );
    for p in [
        ProviderKey::DeepSeek,
        ProviderKey::Gemini,
        ProviderKey::Antigravity,
        ProviderKey::Grok,
    ] {
        assert_eq!(engine_bridge::login_command(p), None, "{p:?}");
    }
}

/// `(process variable, env file, state directory, expected path)`.
type Row<'a> = (
    Option<String>,
    Option<&'a Path>,
    Option<PathBuf>,
    Option<PathBuf>,
);

#[test]
fn usage_log_path_takes_its_three_sources_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let with_key = dir.path().join("with-key.env");
    std::fs::write(
        &with_key,
        "OTHER=1\nexport JEV_USAGE_LOG=\"/from/file/usage.jsonl\"\n",
    )
    .unwrap();
    let without_key = dir.path().join("without-key.env");
    std::fs::write(
        &without_key,
        "DEEPSEEK_API_KEY=JEVCANARY_KEY_x\n# JEV_USAGE_LOG=/commented\n",
    )
    .unwrap();
    let empty_key = dir.path().join("empty-key.env");
    std::fs::write(&empty_key, "JEV_USAGE_LOG=\n").unwrap();
    let absent = dir.path().join("absent.env");
    let state = PathBuf::from("/state/jev-router");
    let env = |s: &str| Some(s.to_owned());
    let path = |s: &str| Some(PathBuf::from(s));

    // (process variable, env file, state directory) -> expected; the engine's order:
    // a non-empty process variable, else the env-file key, else <state_dir>/usage.jsonl.
    let table: Vec<Row<'_>> = vec![
        (
            env("/from/env.jsonl"),
            Some(&with_key),
            Some(state.clone()),
            path("/from/env.jsonl"),
        ),
        (env("/from/env.jsonl"), None, None, path("/from/env.jsonl")),
        (
            env("relative.jsonl"),
            Some(&with_key),
            Some(state.clone()),
            path("relative.jsonl"),
        ),
        (
            None,
            Some(&with_key),
            Some(state.clone()),
            path("/from/file/usage.jsonl"),
        ),
        (
            env(""),
            Some(&with_key),
            Some(state.clone()),
            path("/from/file/usage.jsonl"),
        ),
        (None, Some(&with_key), None, path("/from/file/usage.jsonl")),
        (
            None,
            Some(&without_key),
            Some(state.clone()),
            path("/state/jev-router/usage.jsonl"),
        ),
        (
            None,
            Some(&empty_key),
            Some(state.clone()),
            path("/state/jev-router/usage.jsonl"),
        ),
        (
            None,
            Some(&absent),
            Some(state.clone()),
            path("/state/jev-router/usage.jsonl"),
        ),
        (
            None,
            None,
            Some(state.clone()),
            path("/state/jev-router/usage.jsonl"),
        ),
        (
            env(""),
            None,
            Some(state),
            path("/state/jev-router/usage.jsonl"),
        ),
        (None, Some(&without_key), None, None),
        (env(""), Some(&absent), None, None),
        (None, None, None, None),
    ];
    for (index, (variable, file, state_dir, expected)) in table.into_iter().enumerate() {
        assert_eq!(
            engine_bridge::usage_log_path_from(variable, file, state_dir),
            expected,
            "row {index}"
        );
    }
}

// ---- the real environment, in a child process -------------------------------------------

/// The variable that turns the test below into the child's body.
const CHILD_MARKER: &str = "FLASHBOARD_BRIDGE_CHILD";

fn show(path: Option<PathBuf>) -> String {
    path.map_or_else(|| "<none>".to_owned(), |p| p.to_str().unwrap().to_owned())
}

/// The child's body: prints what the port says under the environment it was given. As a
/// normal test (no marker) it does nothing.
#[test]
fn child_reports_the_port() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    // The harness prints the test's name without a newline first.
    let mut out = String::from("\n");
    writeln!(out, "FB env_file={}", show(engine_bridge::env_file())).unwrap();
    writeln!(
        out,
        "FB usage_log_path={}",
        show(engine_bridge::usage_log_path())
    )
    .unwrap();
    writeln!(out, "FB state_dir={}", show(engine_bridge::state_dir())).unwrap();
    writeln!(
        out,
        "FB dashboard_dir={}",
        show(engine_bridge::dashboard_dir())
    )
    .unwrap();
    std::io::Write::write_all(&mut std::io::stdout(), out.as_bytes()).unwrap();
}

/// Re-executes this test binary with exactly `vars` as its environment and returns what
/// the child reported.
fn in_child(vars: &[(&str, &Path)]) -> BTreeMap<String, String> {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "child_reports_the_port",
        "--exact",
        "--nocapture",
        "--test-threads=1",
    ]);
    command.env_clear().env(CHILD_MARKER, "1");
    for (name, value) in vars {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "the child failed");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("FB "))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn state_directory_follows_xdg_state_home() {
    let dir = tempfile::tempdir().unwrap();
    let (xdg, home) = (dir.path().join("xdg"), dir.path().join("home"));
    let report = in_child(&[("XDG_STATE_HOME", &xdg), ("HOME", &home)]);
    let expected = xdg.join("jev-router");
    assert_eq!(report["state_dir"], expected.to_str().unwrap());
    assert!(expected.is_dir(), "the engine creates the directory");
    assert_eq!(mode(&expected), 0o700, "owner-only");
    assert!(
        !home.exists(),
        "HOME is not touched when XDG_STATE_HOME is set"
    );
    assert_eq!(
        report["usage_log_path"],
        expected.join("usage.jsonl").to_str().unwrap()
    );
    // A test build is a debug build: the development directory (DESIGN 8.8).
    assert_eq!(
        report["dashboard_dir"],
        expected.join("dashboard-dev").to_str().unwrap()
    );
    assert!(
        !expected.join("dashboard-dev").exists(),
        "the port does not create Flashboard's directory"
    );
}

#[test]
fn state_directory_falls_back_to_home() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let expected = home.join(".local/state/jev-router");
    let report = in_child(&[("HOME", &home)]);
    assert_eq!(report["state_dir"], expected.to_str().unwrap());
    assert!(expected.is_dir());
    // An empty XDG_STATE_HOME counts as unset.
    let report = in_child(&[("HOME", &home), ("XDG_STATE_HOME", Path::new(""))]);
    assert_eq!(report["state_dir"], expected.to_str().unwrap());
}

#[test]
fn without_home_there_is_no_state_directory_and_no_env_file() {
    let report = in_child(&[]);
    assert_eq!(report["state_dir"], "<none>");
    assert_eq!(report["dashboard_dir"], "<none>");
    assert_eq!(report["env_file"], "<none>");
    assert_eq!(report["usage_log_path"], "<none>");
}

#[test]
fn env_file_path_with_and_without_its_override() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let report = in_child(&[("HOME", &home)]);
    assert_eq!(
        report["env_file"],
        home.join(".jev-router.env").to_str().unwrap()
    );
    let custom = dir.path().join("elsewhere/keys.env");
    let report = in_child(&[("HOME", &home), ("JEV_ROUTER_ENV", &custom)]);
    assert_eq!(report["env_file"], custom.to_str().unwrap());
    let report = in_child(&[("JEV_ROUTER_ENV", &custom)]);
    assert_eq!(
        report["env_file"],
        custom.to_str().unwrap(),
        "the override needs no HOME"
    );
    assert!(
        !custom.exists() && !home.join(".jev-router.env").exists(),
        "the path is computed, never created"
    );
}

#[test]
fn usage_log_path_over_the_real_environment() {
    let dir = tempfile::tempdir().unwrap();
    let (home, xdg) = (dir.path().join("home"), dir.path().join("xdg"));
    std::fs::create_dir_all(&home).unwrap();

    // 1. The process variable wins, and the state directory is not even created.
    let from_env = dir.path().join("env/usage.jsonl");
    let report = in_child(&[
        ("HOME", &home),
        ("XDG_STATE_HOME", &xdg),
        ("JEV_USAGE_LOG", &from_env),
    ]);
    assert_eq!(report["usage_log_path"], from_env.to_str().unwrap());

    // 2. Else the key in the env file (the default file under HOME, then the override).
    let from_file = dir.path().join("file/usage.jsonl");
    std::fs::write(
        home.join(".jev-router.env"),
        format!("export JEV_USAGE_LOG='{}'\n", from_file.display()),
    )
    .unwrap();
    let report = in_child(&[("HOME", &home), ("XDG_STATE_HOME", &xdg)]);
    assert_eq!(report["usage_log_path"], from_file.to_str().unwrap());
    let report = in_child(&[
        ("HOME", &home),
        ("XDG_STATE_HOME", &xdg),
        ("JEV_USAGE_LOG", Path::new("")),
    ]);
    assert_eq!(
        report["usage_log_path"],
        from_file.to_str().unwrap(),
        "an empty variable counts as unset"
    );
    let other_file = dir.path().join("other.env");
    let from_other = dir.path().join("other/usage.jsonl");
    std::fs::write(
        &other_file,
        format!("JEV_USAGE_LOG={}\n", from_other.display()),
    )
    .unwrap();
    let report = in_child(&[
        ("HOME", &home),
        ("XDG_STATE_HOME", &xdg),
        ("JEV_ROUTER_ENV", &other_file),
    ]);
    assert_eq!(report["usage_log_path"], from_other.to_str().unwrap());

    // 3. Else <state_dir>/usage.jsonl.
    std::fs::write(
        home.join(".jev-router.env"),
        "DEEPSEEK_API_KEY=JEVCANARY_KEY_x\n",
    )
    .unwrap();
    let report = in_child(&[("HOME", &home), ("XDG_STATE_HOME", &xdg)]);
    assert_eq!(
        report["usage_log_path"],
        xdg.join("jev-router/usage.jsonl").to_str().unwrap()
    );
    assert!(
        !report.values().any(|v| v.contains("JEVCANARY")),
        "no key leaves the env file"
    );
}

// ---- the engine is named in two lines ----------------------------------------------------

fn dashboard() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn files(dir: &Path, wanted: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        if path.is_dir() {
            if !["node_modules", "target", "dist", "gen", ".git"].contains(&name.as_str()) {
                files(&path, wanted, out);
            }
        } else if wanted(&path) {
            out.push(path);
        }
    }
}

/// `(file relative to dashboard/, line)` for every line of the chosen files with a needle.
fn hits(wanted: &dyn Fn(&Path) -> bool, needles: &[String]) -> Vec<(String, String)> {
    let root = dashboard().canonicalize().unwrap();
    let mut found = Vec::new();
    let mut paths = Vec::new();
    files(&root, wanted, &mut paths);
    paths.sort();
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        for line in text.lines() {
            if needles.iter().any(|needle| line.contains(needle.as_str())) {
                found.push((
                    path.strip_prefix(&root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                    line.trim().to_owned(),
                ));
            }
        }
    }
    found
}

#[test]
fn the_engine_is_named_in_exactly_two_lines() {
    // Built in halves, so this file does not contain the identifiers it searches for.
    let identifiers = [
        format!("jev_{}", "engine"),
        format!("jev_{}", "router"),
        format!("jev_{}", "embed"),
    ];
    let rust = hits(&|p| p.extension().is_some_and(|e| e == "rs"), &identifiers);
    let import_line = format!("use jev_{} as engine;", "engine");
    assert_eq!(
        rust,
        [
            (
                "crates/chat-io/src/engine_bridge.rs".to_owned(),
                import_line.clone()
            ),
            (
                "crates/usage-io/src/engine_bridge.rs".to_owned(),
                import_line
            ),
        ],
        "only each io crate's engine_bridge.rs may name the engine, in its one import line"
    );

    let packages = [
        format!("jev-{}", "engine"),
        format!("jev-{}", "router"),
        format!("jev-{}", "embed"),
    ];
    let manifests = hits(
        &|p| p.file_name().is_some_and(|n| n == "Cargo.toml"),
        &packages,
    );
    assert_eq!(manifests.len(), 2, "{manifests:?}");
    assert_eq!(manifests[0].0, "crates/chat-io/Cargo.toml");
    assert_eq!(manifests[1].0, "crates/usage-io/Cargo.toml");
    for entry in &manifests {
        assert!(
            entry
                .1
                .starts_with(&format!("jev-{} = {{ package = ", "engine")),
            "{}",
            entry.1
        );
    }
}
