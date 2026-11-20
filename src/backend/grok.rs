//! xAI Grok Build backend: `grok --single=<prompt> --output-format streaming-messages-json`.
//!
//! Written from the official docs only, never against a recorded run (see
//! `tests/fixtures/grok/README.md`). Two choices follow from the docs:
//! - Headless Grok does not read stdin, so the prompt travels in the args. A prompt
//!   too long for one argument goes through the documented `--prompt-file <PATH>`
//!   instead (user guide 14-headless-mode.md), see [`PromptFiles`].
//! - `streaming-messages-json` is used instead of `streaming-json` because only it
//!   has one line holding the whole answer (`result`). `streaming-json` sends the
//!   answer as `text` fragments and its `end` line has no text, which a stateless
//!   [`Backend::parse_line`] cannot assemble without sharing state between turns.

use std::path::Path;

use serde_json::Value;
use tokio::process::Command;

use crate::mode::Mode;

use super::claude::tool_summary;
use super::{
    classify_common, has_any, Backend, ErrorKind, Event, Finish, PromptArg, PromptFiles, ProviderId, TurnRequest, Usage,
};

/// Permission mode for headless turns. Never `bypassPermissions` or
/// `--always-approve`: a routed turn must not get blanket tool access.
const PERMISSION_MODE: &str = "default";
/// Tools allowed without a prompt in Edit and Auto only. `Write` is an alias of
/// `Edit`; Ask and Plan receive no allow rules.
const ALLOWED_TOOLS: [&str; 2] = ["Edit", "Write"];

/// The variable the Grok CLI reads an xAI API key from, as an alternative to `grok login`.
pub const API_KEY_ENV: &str = "XAI_API_KEY";

pub struct GrokBackend {
    pub bin: String,
    /// Prompt files of the current turn; one instance serves one turn.
    prompts: PromptFiles,
    /// Variables the child needs that may live only in `~/.jev-router.env` (the API key),
    /// which the child process would not otherwise see.
    extra_env: Vec<(String, String)>,
}

impl GrokBackend {
    pub fn new(bin: impl Into<String>) -> Self {
        Self { bin: bin.into(), prompts: PromptFiles::default(), extra_env: Vec::new() }
    }

    /// Pass these variables to the CLI (see `extra_env`).
    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.extra_env = env;
        self
    }

    /// Grok Build with the user's own login or `XAI_API_KEY`; the binary can be
    /// overridden with `GROK_BIN`.
    pub fn grok() -> Self {
        Self::new(crate::envfile::lookup("GROK_BIN").unwrap_or_else(|| "grok".into()))
    }
}

/// `XAI_API_KEY` when it is set in the environment OR in `~/.jev-router.env`, so a key
/// kept only in that file still reaches the CLI (and the probe agrees).
pub fn auth_env(lookup: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    lookup(API_KEY_ENV).filter(|v| !v.trim().is_empty()).map(|v| (API_KEY_ENV.to_string(), v)).into_iter().collect()
}

/// Whether Grok has ever stored state for this user (`<home>/.grok`). Lets an
/// availability probe guess "installed and used" without running the binary,
/// which is a lazy installer on some machines. `GROK_HOME` is not consulted.
pub fn session_dir_exists(home: &Path) -> bool {
    home.join(".grok").exists()
}

fn count(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

/// Grok's `input_tokens` is the uncached share only, so the total adds both cache
/// buckets. The context size is not reported per call here, hence `None`.
fn usage_of(result: &Value) -> Usage {
    let u = &result["usage"];
    let cached = count(u, "cache_read_input_tokens");
    Usage {
        input_tokens: count(u, "input_tokens") + cached + count(u, "cache_creation_input_tokens"),
        cached_input_tokens: cached,
        output_tokens: count(u, "output_tokens"),
        context_tokens: None,
    }
}

fn assistant_events(ev: &Value) -> Vec<Event> {
    let blocks = ev["message"]["content"].as_array().into_iter().flatten();
    blocks
        .filter_map(|b| match b["type"].as_str() {
            Some("tool_use") => Some(Event::Tool(tool_summary(b["name"].as_str().unwrap_or("?"), &b["input"]))),
            Some("text") => {
                let t = b["text"].as_str().unwrap_or("").trim();
                (!t.is_empty()).then(|| Event::Text(t.to_string()))
            }
            _ => None,
        })
        .collect()
}

/// Why a `result` failed: its `errors[]`, else its text, else its subtype.
fn failure_message(ev: &Value) -> String {
    let errors: Vec<String> = ev["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| e.as_str().map_or_else(|| e.to_string(), String::from))
        .collect();
    let text = ev["result"].as_str().unwrap_or("").trim();
    if !errors.is_empty() {
        errors.join("; ")
    } else if !text.is_empty() {
        text.to_string()
    } else {
        ev["subtype"].as_str().map_or_else(|| "grok reported an error".into(), |s| format!("grok failed: {s}"))
    }
}

fn result_event(ev: &Value) -> Event {
    if ev["is_error"].as_bool() == Some(true) {
        return Event::Failed { message: failure_message(ev) };
    }
    Event::Finished(Finish {
        text: ev["result"].as_str().unwrap_or("").to_string(),
        session_id: ev["session_id"].as_str().map(String::from),
        usage: usage_of(ev),
    })
}

/// The plan gate shows up as a refusal that names Grok's plan, not as a limit. Phrases
/// only: "subscription", "premium" and "required" appear together in ordinary billing
/// code ("error: subscription id is required"). Wording that is only a gate in a Grok
/// context ("subscription is required", "not included in your") needs Grok or X Premium
/// named as well.
fn is_entitlement_text(lower: &str) -> bool {
    const ALWAYS: [&str; 4] = ["requires a supergrok", "supergrok subscription", "upgrade to supergrok", "x premium+"];
    const NEEDS_GROK: [&str; 6] = [
        "not included in your",
        "subscription is required",
        "subscription required",
        "no active subscription",
        "upgrade to",
        "only available to",
    ];
    has_any(lower, &ALWAYS) || (has_any(lower, &["grok", "x premium"]) && has_any(lower, &NEEDS_GROK))
}

/// Grok's own login wording; the shared rules cover "not logged in", "invalid api key"
/// and "authentication failed|required". "xai_api_key" counts only next to a word that
/// says it is missing or wrong, never on its own (docs and config mention it freely).
fn is_auth_text(lower: &str) -> bool {
    let key_broken = has_any(lower, &["missing", "not set", "invalid", "not found", "is required"]);
    has_any(lower, &["grok login", "not authenticated", "api key is missing", "api key is invalid", "api key not set"])
        || (lower.contains("xai_api_key") && key_broken)
}

impl Backend for GrokBackend {
    fn provider(&self) -> ProviderId {
        ProviderId::Grok
    }

    fn command(&self, req: &TurnRequest) -> Command {
        let mut cmd = Command::new(&self.bin);
        // `--single=` (not `-p <prompt>`) so a prompt that starts with `-` is not read as a flag.
        match self.prompts.arg(&super::prompt_with_images(req)) {
            PromptArg::Inline(prompt) => cmd.arg(format!("--single={prompt}")),
            PromptArg::File { path, .. } => cmd.arg("--prompt-file").arg(path),
        };
        cmd.args(["--model", &req.model, "--output-format", "streaming-messages-json"])
            .args(["--no-auto-update", "--no-alt-screen", "--permission-mode", PERMISSION_MODE])
            .current_dir(&req.dir);
        if matches!(req.mode, Mode::Edit | Mode::Auto) {
            for tool in ALLOWED_TOOLS {
                cmd.args(["--allow", tool]);
            }
        }
        if let Some(id) = &req.session_id {
            cmd.args(["--resume", id]);
        }
        cmd.envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        cmd
    }

    /// Headless Grok ignores piped stdin; the prompt is in the args.
    fn stdin_payload(&self, _req: &TurnRequest) -> Option<String> {
        None
    }

    fn cleanup(&self) {
        self.prompts.cleanup();
    }

    fn parse_line(&self, line: &str) -> Vec<Event> {
        let Ok(ev) = serde_json::from_str::<Value>(line) else { return vec![] };
        match ev["type"].as_str() {
            Some("assistant") => assistant_events(&ev),
            Some("result") => vec![result_event(&ev)],
            // Documented for every output format, e.g. "Couldn't start session: ...".
            Some("error") => vec![Event::Failed { message: ev["message"].as_str().unwrap_or("grok reported an error").into() }],
            _ => vec![],
        }
    }

    fn classify_error(&self, message: &str) -> ErrorKind {
        if let Some(kind) = classify_common(message) {
            return kind;
        }
        let lower = message.to_lowercase();
        if is_entitlement_text(&lower) {
            ErrorKind::NotEntitled
        } else if is_auth_text(&lower) {
            ErrorKind::Auth
        } else {
            ErrorKind::Other
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURN1: &str = fixture!("grok/turn1.jsonl");
    const ERR_AUTH: &str = fixture!("grok/err_auth.jsonl");
    const ERR_PLAN: &str = fixture!("grok/err_not_entitled.jsonl");
    const EMPTY: &str = fixture!("grok/empty.jsonl");

    fn backend() -> GrokBackend {
        GrokBackend::new("grok")
    }

    fn parse_all(fixture: &str) -> Vec<Event> {
        fixture.lines().flat_map(|l| backend().parse_line(l)).collect()
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn req(session: Option<&str>) -> TurnRequest {
        TurnRequest {
            dir: "/tmp".into(),
            model: "grok-4.6".into(),
            prompt: "hi".into(),
            images: vec![],
            session_id: session.map(String::from),
            mode: Default::default(),
        }
    }

    #[test]
    fn normal_turn_parses_to_tool_text_finish() {
        let events = parse_all(TURN1);
        assert_eq!(events.len(), 3, "{events:?}");
        assert_eq!(events[0], Event::Tool("read_file /tmp/spike/a.txt".into()));
        assert_eq!(events[1], Event::Text("The file says hello.".into()));
        let Event::Finished(f) = &events[2] else { panic!("{:?}", events[2]) };
        assert_eq!(f.text, "The file says hello.");
        assert_eq!(f.session_id.as_deref(), Some("0199c0de-7a3e-7c21-9b8f-3d5e6f7a8b9c"));
        // 932 uncached + 1500 cache reads + 200 cache writes.
        assert_eq!(
            f.usage,
            Usage { input_tokens: 2632, cached_input_tokens: 1500, output_tokens: 54, context_tokens: None }
        );
    }

    #[test]
    fn auth_failure_is_failed_and_classified_auth() {
        let events = parse_all(ERR_AUTH);
        let [Event::Failed { message }] = &events[..] else { panic!("{events:?}") };
        assert!(message.contains("grok login"));
        assert_eq!(backend().classify_error(message), ErrorKind::Auth);
    }

    #[test]
    fn plan_refusal_is_not_entitled() {
        let events = parse_all(ERR_PLAN);
        let [Event::Failed { message }] = &events[..] else { panic!("{events:?}") };
        assert!(message.contains("SuperGrok"));
        assert_eq!(backend().classify_error(message), ErrorKind::NotEntitled);
        assert_eq!(backend().classify_error("A SuperGrok subscription is required"), ErrorKind::NotEntitled);
    }

    #[test]
    fn empty_answer_still_finishes_so_the_runner_can_call_it_empty() {
        let events = parse_all(EMPTY);
        let [Event::Finished(f)] = &events[..] else { panic!("{events:?}") };
        assert!(f.text.is_empty());
        assert_eq!(f.usage.output_tokens, 0);
    }

    #[test]
    fn failure_message_falls_back_to_text_then_subtype() {
        let msg = |v: &str| match &backend().parse_line(v)[..] {
            [Event::Failed { message }] => message.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(msg(r#"{"type":"result","is_error":true,"result":"boom"}"#), "boom");
        assert_eq!(msg(r#"{"type":"result","is_error":true,"subtype":"error_max_turns"}"#), "grok failed: error_max_turns");
        assert_eq!(msg(r#"{"type":"result","is_error":true}"#), "grok reported an error");
    }

    #[test]
    fn junk_and_housekeeping_lines_are_ignored() {
        assert!(backend().parse_line("not json").is_empty());
        assert!(backend().parse_line("").is_empty());
        assert!(backend().parse_line(r#"{"type":"system","subtype":"init"}"#).is_empty());
        assert!(backend().parse_line(r#"{"type":"user","message":{"content":[]}}"#).is_empty());
    }

    #[test]
    fn classifies_common_errors_and_unknowns() {
        assert!(matches!(backend().classify_error("HTTP 429 Too Many Requests"), ErrorKind::Quota { .. }));
        assert_eq!(backend().classify_error("weird"), ErrorKind::Other);
    }

    #[test]
    fn golden_grok_args() {
        const MASKED: &str = "# masked: nothing";
        const GOLDEN: &str = golden!("backend/grok_args.txt");
        let case = |name: &str, r: TurnRequest| format!("=== {name}\n{}\n", serde_json::to_string(&args(&backend().command(&r))).unwrap());
        let actual = [
            case("fresh_edit", req(None)),
            case("resumed_edit", req(Some("sess-1"))),
            case("fresh_ask", TurnRequest { mode: Mode::Ask, ..req(None) }),
        ]
        .concat();
        let (head, body) = GOLDEN.split_once('\n').unwrap();
        assert_eq!(head, MASKED);
        assert_eq!(body, actual);
    }

    #[test]
    fn command_is_headless_with_scoped_permissions() {
        for mode in Mode::ALL {
            let mut expected = vec!["--single=hi", "--model", "grok-4.6", "--output-format", "streaming-messages-json", "--no-auto-update", "--no-alt-screen", "--permission-mode", "default"];
            if matches!(mode, Mode::Edit | Mode::Auto) {
                expected.extend(["--allow", "Edit", "--allow", "Write"]);
            }
            assert_eq!(args(&backend().command(&TurnRequest { mode, ..req(None) })), expected);
            expected.extend(["--resume", "abc"]);
            assert_eq!(args(&backend().command(&TurnRequest { mode, ..req(Some("abc")) })), expected);
        }
    }

    #[test]
    fn extra_env_reaches_the_child_so_a_key_kept_in_the_env_file_works() {
        let b = GrokBackend::new("grok").with_env(vec![("XAI_API_KEY".into(), "k".into())]);
        let cmd = b.command(&req(None));
        assert!(cmd.as_std().get_envs().any(|(k, v)| k == "XAI_API_KEY" && v == Some(std::ffi::OsStr::new("k"))));
    }

    #[test]
    fn auth_env_picks_up_a_non_empty_key_only() {
        let lookup = |n: &str| (n == "XAI_API_KEY").then(|| "from-file".to_string());
        assert_eq!(auth_env(&lookup), vec![("XAI_API_KEY".to_string(), "from-file".to_string())]);
        assert!(auth_env(&|_| None).is_empty());
        assert!(auth_env(&|_| Some("  ".to_string())).is_empty());
    }

    #[test]
    fn command_never_bypasses_permissions() {
        let all = args(&backend().command(&req(Some("abc")))).join(" ").to_lowercase();
        for banned in ["always-approve", "yolo", "bypasspermissions"] {
            assert!(!all.contains(banned), "{banned} in {all}");
        }
    }

    #[test]
    fn prompt_travels_in_args_not_stdin() {
        let r = TurnRequest { prompt: "--looks-like-a-flag".into(), ..req(None) };
        assert_eq!(backend().stdin_payload(&r), None);
        assert_eq!(args(&backend().command(&r))[0], "--single=--looks-like-a-flag");
    }

    #[test]
    fn a_huge_prompt_goes_through_prompt_file_and_no_argument_is_over_the_limit() {
        let dir = std::env::temp_dir().join(format!("jev-grok-prompt-{}", std::process::id()));
        let mut b = backend();
        b.prompts = PromptFiles::in_dir(dir.clone());
        let prompt = "q".repeat(500_000);
        let all = args(&b.command(&TurnRequest { prompt: prompt.clone(), ..req(None) }));
        assert!(all.iter().all(|a| a.len() <= crate::backend::MAX_PROMPT_ARG_BYTES), "an argument is too long");
        assert_eq!(all[0], "--prompt-file");
        let path = std::path::PathBuf::from(&all[1]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), prompt);
        assert!(!all.iter().any(|a| a.starts_with("--single")));
        b.cleanup();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn provider_is_grok() {
        assert_eq!(backend().provider(), ProviderId::Grok);
    }

    #[test]
    fn session_dir_probe_checks_dot_grok_under_home() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let home = std::env::temp_dir().join(format!("jev-grok-test-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        assert!(!session_dir_exists(&home));
        std::fs::create_dir(home.join(".grok")).unwrap();
        assert!(session_dir_exists(&home));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn billing_words_alone_do_not_mean_the_plan_gate_or_a_login_problem() {
        assert!(!is_entitlement_text("update the subscription handler to apply premium pricing"));
        assert!(!is_auth_text("the api key field is rendered in the settings form"));
    }

    #[test]
    fn entitlement_needs_a_plan_phrase_not_common_word_pairs() {
        for text in [
            "error: subscription id is required",
            "premium pricing handler",
            "the node version is old and a subscription is required by the billing api",
            "the file is not included in your tsconfig",
            "your plan is to upgrade the subscription module",
        ] {
            assert!(!is_entitlement_text(&text.to_lowercase()), "{text:?}");
            assert_eq!(backend().classify_error(text), ErrorKind::Other, "{text:?}");
        }
        for text in [
            "Grok Build is not included in your X Premium plan. A SuperGrok or X Premium+ subscription is required.",
            "Grok requires a SuperGrok plan",
            "Upgrade to SuperGrok to use grok build",
            "A SuperGrok subscription is required",
            "grok: subscription required",
            "No active subscription for Grok",
        ] {
            assert!(is_entitlement_text(&text.to_lowercase()), "{text:?}");
            assert_eq!(backend().classify_error(text), ErrorKind::NotEntitled, "{text:?}");
        }
    }

    #[test]
    fn auth_needs_a_login_phrase_not_common_word_pairs() {
        for text in [
            "api key validation error",
            "the xai_api_key variable is documented in the readme",
            "authentication module has an error",
            "api key rotation failed in the billing job",
        ] {
            assert!(!is_auth_text(&text.to_lowercase()), "{text:?}");
            assert_eq!(backend().classify_error(text), ErrorKind::Other, "{text:?}");
        }
        for text in [
            "Couldn't start session: not authenticated. Run `grok login` or set XAI_API_KEY.",
            "api key missing: run grok login",
            "XAI_API_KEY is not set",
            "XAI_API_KEY invalid",
            "the Grok API key is missing",
        ] {
            assert!(is_auth_text(&text.to_lowercase()) || classify_common(text).is_some(), "{text:?}");
            assert_eq!(backend().classify_error(text), ErrorKind::Auth, "{text:?}");
        }
    }
}
