//! xAI Grok Build backend: `grok --single=<prompt> --output-format streaming-messages-json`.
//!
//! Written from the official docs only, never against a recorded run (see
//! `tests/fixtures/grok/README.md`). Two choices follow from the docs:
//! - Headless Grok does not read stdin, so the prompt travels in the args.
//! - `streaming-messages-json` is used instead of `streaming-json` because only it
//!   has one line holding the whole answer (`result`). `streaming-json` sends the
//!   answer as `text` fragments and its `end` line has no text, which a stateless
//!   [`Backend::parse_line`] cannot assemble without sharing state between turns.

use std::path::Path;

use serde_json::Value;
use tokio::process::Command;

use super::claude::tool_summary;
use super::{classify_common, Backend, ErrorKind, Event, Finish, ProviderId, TurnRequest, Usage};

/// Permission mode for headless turns. Never `bypassPermissions` or
/// `--always-approve`: a routed turn must not get blanket tool access.
const PERMISSION_MODE: &str = "default";
/// Tools allowed without a prompt. In `-p` nobody can answer a prompt, so file
/// edits need an explicit rule (`Write` is an alias of `Edit`, listed for clarity).
const ALLOWED_TOOLS: [&str; 2] = ["Edit", "Write"];

pub struct GrokBackend {
    pub bin: String,
}

impl GrokBackend {
    /// Grok Build with the user's own login; the binary can be overridden with `GROK_BIN`.
    pub fn grok() -> Self {
        Self { bin: std::env::var("GROK_BIN").unwrap_or_else(|_| "grok".into()) }
    }
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

/// The plan gate shows up as a refusal that names the subscription, not as a limit.
fn is_entitlement_text(lower: &str) -> bool {
    ["not included", "subscription", "supergrok", "premium"].iter().any(|p| lower.contains(p))
}

/// Grok's own login wording; the shared rules only know "authenticate".
fn is_auth_text(lower: &str) -> bool {
    ["grok login", "xai_api_key", "authentication", "api key"].iter().any(|p| lower.contains(p))
}

impl Backend for GrokBackend {
    fn provider(&self) -> ProviderId {
        ProviderId::Grok
    }

    fn command(&self, req: &TurnRequest) -> Command {
        let mut cmd = Command::new(&self.bin);
        // `--single=` (not `-p <prompt>`) so a prompt that starts with `-` is not read as a flag.
        cmd.arg(format!("--single={}", req.prompt))
            .args(["--model", &req.model, "--output-format", "streaming-messages-json"])
            .args(["--no-auto-update", "--no-alt-screen", "--permission-mode", PERMISSION_MODE])
            .current_dir(&req.dir);
        for tool in ALLOWED_TOOLS {
            cmd.args(["--allow", tool]);
        }
        if let Some(id) = &req.session_id {
            cmd.args(["--resume", id]);
        }
        cmd
    }

    /// Headless Grok ignores piped stdin; the prompt is in the args.
    fn stdin_payload(&self, _req: &TurnRequest) -> Option<String> {
        None
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

    const TURN1: &str = include_str!("../../tests/fixtures/grok/turn1.jsonl");
    const ERR_AUTH: &str = include_str!("../../tests/fixtures/grok/err_auth.jsonl");
    const ERR_PLAN: &str = include_str!("../../tests/fixtures/grok/err_not_entitled.jsonl");
    const EMPTY: &str = include_str!("../../tests/fixtures/grok/empty.jsonl");

    fn backend() -> GrokBackend {
        GrokBackend { bin: "grok".into() }
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
            session_id: session.map(String::from),
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
        assert_eq!(backend().classify_error("A subscription is required"), ErrorKind::NotEntitled);
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
    fn command_is_headless_with_scoped_permissions() {
        assert_eq!(
            args(&backend().command(&req(None))),
            ["--single=hi", "--model", "grok-4.6", "--output-format", "streaming-messages-json", "--no-auto-update",
             "--no-alt-screen", "--permission-mode", "default", "--allow", "Edit", "--allow", "Write"]
        );
        let resumed = args(&backend().command(&req(Some("abc"))));
        assert_eq!(&resumed[resumed.len() - 2..], ["--resume", "abc"]);
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
}
