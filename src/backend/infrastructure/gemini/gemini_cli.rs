//! Gemini backend. Google ships two headless CLIs for Gemini models and a user may
//! have either logged in: `gemini` (Gemini CLI) and `agy` (Antigravity CLI, which
//! replaced Gemini CLI for free-tier and Google One users on 2026-06-18). Both are
//! one provider ([`ProviderId::Gemini`]), so one struct serves both and only the
//! flags and event shapes that genuinely differ branch on [`Flavor`].
//!
//! Gemini CLI uses headless stream-json. Antigravity uses ACP so tool approvals
//! reach the host instead of being silently denied by print mode. The legacy
//! headless fixtures remain as regression evidence; see `tests/fixtures/agy/README.md`.


use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::Value;
use tokio::process::Command;

use crate::approval::domain::mode::Mode;

use crate::backend::infrastructure::claude::claude_cli::tool_summary;
use crate::backend::{
    classify_common, Backend, Decision, ErrorKind, Event, Finish, PermissionRequest, ProviderId, TurnRequest, Usage,
};

/// Which of Google's two CLIs a [`GeminiBackend`] drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    GeminiCli,
    Antigravity,
}

/// Gemini CLI's edit-approval mode, used only for Edit and Auto turns. Ask and
/// Plan use `default`; no turn uses an approve-everything mode such as `--yolo`.
const GEMINI_APPROVAL_MODE: &str = "auto_edit";

/// What a blank answer is called, so [`classify_text`] can map it to `Empty`.
const EMPTY_ANSWER: &str = "the CLI returned an empty response";

/// Environment variables that make Gemini CLI authenticate without `settings.json`
/// (read from its `getAuthTypeFromEnv`). Only [`is_configured`] reads this list.
pub const AUTH_ENV_VARS: [&str; 4] =
    ["GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_GENAI_USE_VERTEXAI", "GOOGLE_GENAI_USE_GCA"];

/// What the current turn has said so far. Gemini CLI's `result` event carries no
/// answer text, only stats, so the answer must be rebuilt from the `message`
/// deltas that came before it.
#[derive(Default)]
struct Pending {
    /// Assistant text since the last tool call (the final answer once `result` arrives).
    text: String,
    session_id: Option<String>,
    /// Last `error` event, kept because an empty reply ends with a bare error `result`.
    notice: Option<String>,
}

/// A Gemini CLI or Antigravity CLI turn runner.
///
/// Holds per-turn parse state (Gemini CLI streams the answer in pieces, and
/// [`Backend::parse_line`] takes `&self`), so use one instance per turn: a second
/// turn running at the same time on the same instance would mix their text. The
/// constructors are cheap. Build with [`GeminiBackend::new`] or the `gemini_cli` /
/// `antigravity` constructors, not a struct literal.
pub struct GeminiBackend {
    pub flavor: Flavor,
    pub bin: String,
    pending: Mutex<Pending>,
    acp: Mutex<super::acp::Acp>,
    /// Variables the child needs that may live only in `~/.jev-router.env` (an API key),
    /// which the child process would not otherwise see.
    extra_env: Vec<(String, String)>,
}

impl GeminiBackend {
    pub fn new(flavor: Flavor, bin: impl Into<String>) -> Self {
        Self {
            flavor,
            bin: bin.into(),
            pending: Mutex::new(Pending::default()),
            acp: Mutex::default(),
            extra_env: Vec::new(),
        }
    }

    /// Pass these variables to the CLI (see `extra_env`).
    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.extra_env = env;
        self
    }

    /// Gemini CLI; the binary is `GEMINI_BIN` else `gemini`.
    pub fn gemini_cli() -> Self {
        Self::new(Flavor::GeminiCli, bin_from_env("GEMINI_BIN", "gemini"))
    }

    /// Antigravity CLI; the binary is `AGY_BIN` else `agy`.
    pub fn antigravity() -> Self {
        Self::new(Flavor::Antigravity, bin_from_env("AGY_BIN", "agy"))
    }

    /// A poisoned lock only means an earlier parse panicked; the state is plain
    /// text and every `init` resets it, so carrying on is safe.
    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn args(&self, req: &TurnRequest) -> Vec<String> {
        match self.flavor {
            Flavor::GeminiCli => gemini_args(req),
            Flavor::Antigravity => vec!["acp".into()],
        }
    }
}

fn bin_from_env(var: &str, default: &str) -> String {
    crate::shared::infrastructure::env_file::lookup(var).unwrap_or_else(|| default.into())
}

/// The prompt travels on stdin (see `stdin_payload`): a piped stdin already makes
/// Gemini CLI headless, and stdin has no size limit where one argv entry does.
fn gemini_args(req: &TurnRequest) -> Vec<String> {
    let approval_mode = match req.mode {
        Mode::Edit | Mode::Auto => GEMINI_APPROVAL_MODE,
        Mode::Ask | Mode::Plan => "default",
    };
    let mut args: Vec<String> = ["-m", &req.model, "--output-format", "stream-json", "--approval-mode", approval_mode]
        .map(String::from)
        .into();
    if let Some(id) = &req.session_id {
        args.extend(["--resume".into(), id.clone()]);
    }
    args
}

fn uint(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(0)
}

/// Gemini's `input_tokens` is the prompt count, which already includes the cached
/// share (`cached`), so it maps straight onto [`Usage`].
fn gemini_usage(stats: &Value) -> Usage {
    Usage {
        input_tokens: uint(stats, "input_tokens"),
        cached_input_tokens: uint(stats, "cached"),
        output_tokens: uint(stats, "output_tokens"),
        context_tokens: None,
    }
}

/// Assumes Antigravity counts like Gemini (input includes cache reads); the `max`
/// keeps the cached share from ever exceeding the total if it does not.
#[cfg(test)]
fn agy_usage(usage: &Value) -> Usage {
    let cached = uint(usage, "cache_read_tokens");
    Usage {
        input_tokens: uint(usage, "input_tokens").max(cached),
        cached_input_tokens: cached,
        output_tokens: uint(usage, "output_tokens"),
        context_tokens: None,
    }
}

/// The shared success rule. Antigravity can report `SUCCESS` with an empty
/// response when it did nothing (google-antigravity/antigravity-cli#1065) and
/// Gemini CLI can end a turn with no text, so a blank answer is a failure, never a
/// `Finished` that the caller would show as a reply.
pub(crate) fn finish_or_empty(text: &str, session_id: Option<String>, usage: Usage, note: &str) -> Event {
    if text.trim().is_empty() {
        return Event::Failed { message: format!("{EMPTY_ANSWER}{note}") };
    }
    Event::Finished(Finish { text: text.to_string(), session_id, usage })
}

/// A tool that failed, shown as a hint: in headless mode a denied tool is the
/// usual reason a turn goes nowhere, and nothing else tells the user.
fn tool_failure(message: Option<&str>) -> Option<Event> {
    let message = message?.trim();
    (!message.is_empty()).then(|| Event::Hint(format!("\u{2717} tool failed: {}", message.chars().take(200).collect::<String>())))
}

impl GeminiBackend {
    /// Gemini CLI `--output-format stream-json`: `init`, `message`, `tool_use`,
    /// `tool_result`, `error` and a final `result`.
    fn gemini_events(&self, ev: &Value) -> Vec<Event> {
        let mut pending = self.pending();
        match ev["type"].as_str() {
            Some("init") => {
                *pending = Pending { session_id: ev["session_id"].as_str().map(String::from), ..Pending::default() };
                vec![]
            }
            Some("message") => {
                if ev["role"] == "assistant" {
                    let text = ev["content"].as_str().unwrap_or("");
                    pending.text.push_str(text);
                    // `delta` messages are pieces of the text being written; the whole text
                    // still comes at the next tool call or in the result.
                    if ev["delta"] == true && !text.is_empty() {
                        return vec![Event::TextDelta(text.to_string())];
                    }
                }
                vec![]
            }
            Some("tool_use") => {
                let tool = tool_summary(ev["tool_name"].as_str().unwrap_or("?"), &ev["parameters"]);
                // Text before a tool call is narration; the answer is what follows the last one.
                let narration = std::mem::take(&mut pending.text);
                let narration = narration.trim();
                let said = (!narration.is_empty()).then(|| Event::Text(narration.to_string()));
                said.into_iter().chain([Event::Tool(tool)]).collect()
            }
            Some("tool_result") if ev["status"] == "error" => {
                tool_failure(ev["error"]["message"].as_str().or(ev["output"].as_str())).into_iter().collect()
            }
            Some("error") => {
                let message = ev["message"].as_str().unwrap_or("").to_string();
                if ev["severity"] == "error" {
                    pending.notice = Some(message.clone());
                }
                vec![Event::Text(format!("! {message}"))]
            }
            Some("result") => vec![gemini_result(ev, std::mem::take(&mut *pending))],
            _ => vec![],
        }
    }
}

/// `error.message` exists only when the CLI died with an exception; a failed
/// generation (empty reply, safety block) leaves just the earlier `error` event.
fn gemini_result(ev: &Value, pending: Pending) -> Event {
    if ev["status"] != "success" {
        let message = ev["error"]["message"].as_str().map(String::from).or(pending.notice);
        return Event::Failed { message: message.unwrap_or_else(|| "gemini reported an error".into()) };
    }
    finish_or_empty(&pending.text, pending.session_id, gemini_usage(&ev["stats"]), "")
}

/// Antigravity `--output-format stream-json`: `init`, `step_update` and a final
/// `result`. A logged-out run is reported to print the bare JSON envelope (no
/// `event` key) instead, so that shape is accepted too.
#[cfg(test)]
fn agy_events(ev: &Value) -> Vec<Event> {
    match ev["event"].as_str() {
        Some("step_update") => agy_step(&ev["step_update"]),
        Some("result") => vec![agy_result(&ev["result"])],
        None if ev.get("status").is_some() => vec![agy_result(ev)],
        _ => vec![],
    }
}

/// Tool steps go `ACTIVE` then `DONE`; announce on `ACTIVE` so each call shows once.
/// Text deltas are skipped: the `result` carries the whole answer.
#[cfg(test)]
fn agy_step(step: &Value) -> Vec<Event> {
    let info = &step["tool_info"];
    match (step["step_type"].as_str(), step["state"].as_str()) {
        (Some("tool"), Some("ACTIVE")) => {
            let name = info["name"].as_str().or(step["tool_name"].as_str()).unwrap_or("tool");
            vec![Event::Tool(tool_summary(name, &info["parameters"]))]
        }
        (Some("tool"), Some("DONE")) => tool_failure(info["error"]["message"].as_str()).into_iter().collect(),
        _ => vec![],
    }
}

#[cfg(test)]
fn agy_result(result: &Value) -> Event {
    let status = result["status"].as_str().unwrap_or("");
    if status != "SUCCESS" {
        let message = result["error"].as_str().map(String::from).unwrap_or_else(|| {
            format!("antigravity ended with status {}", if status.is_empty() { "(none)" } else { status })
        });
        return Event::Failed { message };
    }
    finish_or_empty(
        result["response"].as_str().unwrap_or(""),
        result["conversation_id"].as_str().map(String::from),
        agy_usage(&result["usage"]),
        &denied_note(result),
    )
}

/// An empty answer after denied tool calls is the headless permission trap: the
/// CLI cannot ask, so it denies and still exits 0. Say so instead of just "empty".
#[cfg(test)]
fn denied_note(result: &Value) -> String {
    match result["denied_actions"].as_array().map(Vec::len) {
        Some(n) if n > 0 => format!(
            " ({n} action(s) were denied because headless mode cannot ask; allow them under permissions.allow in the Antigravity settings.json)"
        ),
        _ => String::new(),
    }
}

/// How long until a quota resets, from Gemini's "Your quota will reset after
/// 3h20m5s." (the shared classifier only knows the "resets ..." wording).
fn reset_after(message: &str) -> Option<String> {
    const KEY: &str = "reset after ";
    // ASCII lowercasing keeps byte offsets identical to the original.
    let at = message.to_ascii_lowercase().find(KEY)? + KEY.len();
    let word = message[at..].split_whitespace().next()?.trim_end_matches(['.', ',']);
    (!word.is_empty()).then(|| word.to_string())
}

/// What only these CLIs say: their capacity and auth wording, and a blank answer.
fn classify_text(message: &str) -> Option<ErrorKind> {
    let lower = message.to_lowercase();
    let has = |phrases: &[&str]| phrases.iter().any(|p| lower.contains(p));
    if has(&["resource_exhausted", "exhausted your capacity", "too many requests"]) {
        Some(ErrorKind::Quota { resets_at: None })
    } else if has(&[
        "authentication failed",
        "authentication required",
        "authentication error",
        "not authenticated",
        "auth method",
        "please sign in",
        "sign in to",
        "api key not valid",
        "api_key_invalid",
    ]) {
        Some(ErrorKind::Auth)
    } else if has(&["empty response", "empty answer"]) {
        Some(ErrorKind::Empty)
    } else if has(&["not trusted", "trust_workspace", "trust this folder", "trust this directory"]) {
        Some(ErrorKind::UnusableHere)
    } else {
        None
    }
}

impl Backend for GeminiBackend {
    fn provider(&self) -> ProviderId {
        ProviderId::Gemini
    }

    fn command(&self, req: &TurnRequest) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(self.args(req)).current_dir(&req.dir);
        cmd.envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        cmd
    }

    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        match self.flavor {
            Flavor::GeminiCli => Some(crate::backend::prompt_with_images(req)),
            Flavor::Antigravity => Some(self.acp.lock().unwrap_or_else(PoisonError::into_inner).start(req)),
        }
    }

    fn parse_line(&self, line: &str) -> Vec<Event> {
        let Ok(ev) = serde_json::from_str::<Value>(line) else { return vec![] };
        match self.flavor {
            Flavor::GeminiCli => self.gemini_events(&ev),
            Flavor::Antigravity => self.acp.lock().unwrap_or_else(PoisonError::into_inner).parse(&ev),
        }
    }

    fn interactive(&self) -> bool {
        self.flavor == Flavor::Antigravity
    }

    fn take_responses(&self) -> Vec<String> {
        self.acp.lock().unwrap_or_else(PoisonError::into_inner).drain()
    }

    fn permission_response(&self, req: &PermissionRequest, decision: &Decision) -> Option<String> {
        self.acp.lock().unwrap_or_else(PoisonError::into_inner).answer(req, decision)
            .map(|line| line.trim_end_matches('\n').to_string())
    }

    fn classify_error(&self, message: &str) -> ErrorKind {
        match classify_common(message).or_else(|| classify_text(message)) {
            Some(ErrorKind::Quota { resets_at: None }) => ErrorKind::Quota { resets_at: reset_after(message) },
            Some(kind) => kind,
            None => ErrorKind::Other,
        }
    }
}

/// THE answer to "is the Gemini CLI set up": an API-key or Vertex/Code Assist
/// environment variable (see [`AUTH_ENV_VARS`]; `env_set` says whether a variable
/// is set) or a `settings.json` that names an auth type. The availability probe and
/// the provider factory both ask this, so they cannot disagree. Pure so it can be
/// tested without touching real files or the environment. Antigravity keeps no such
/// setting here, so this says nothing about `agy`.
pub fn is_configured(gemini_settings_json: Option<&str>, env_set: impl Fn(&str) -> bool) -> bool {
    auth_looks_configured(gemini_settings_json, AUTH_ENV_VARS.iter().any(|n| env_set(n)))
}

/// The settings-or-flag half of [`is_configured`]; private, callers go through that.
///
/// Gemini CLI accepts comments in `settings.json`, so whole-line `//` comments are
/// dropped before parsing. Unreadable JSON counts as not configured.
fn auth_looks_configured(gemini_settings_json: Option<&str>, env_has_key: bool) -> bool {
    env_has_key || gemini_settings_json.is_some_and(names_auth_type)
}

fn names_auth_type(json: &str) -> bool {
    let uncommented: String =
        json.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    let Ok(settings) = serde_json::from_str::<Value>(&uncommented) else { return false };
    // `security.auth.selectedType` is current; `selectedAuthType` is the older top-level key.
    let named = settings["security"]["auth"]["selectedType"].as_str().or(settings["selectedAuthType"].as_str());
    named.is_some_and(|t| !t.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEMINI_TURN: &str = fixture!("gemini/turn_tools.jsonl");
    const GEMINI_QUOTA: &str = fixture!("gemini/err_quota.jsonl");
    const GEMINI_EMPTY_ERR: &str = fixture!("gemini/err_empty.jsonl");
    const GEMINI_EMPTY_OK: &str = fixture!("gemini/empty_success.jsonl");
    const GEMINI_AUTH_STDERR: &str = fixture!("gemini/err_auth_stderr.txt");
    const AGY_TURN: &str = fixture!("agy/turn_tools.jsonl");
    const AGY_AUTH: &str = fixture!("agy/err_auth.jsonl");
    const AGY_QUOTA: &str = fixture!("agy/err_quota.jsonl");
    const AGY_EMPTY: &str = fixture!("agy/empty_success.jsonl");
    const AGY_EMPTY_DENIED: &str = fixture!("agy/empty_success_denied.jsonl");

    fn gemini() -> GeminiBackend {
        GeminiBackend::new(Flavor::GeminiCli, "gemini")
    }

    fn agy() -> GeminiBackend {
        GeminiBackend::new(Flavor::Antigravity, "agy")
    }

    fn parse_all(backend: &GeminiBackend, fixture: &str) -> Vec<Event> {
        fixture.lines().flat_map(|l| {
            if backend.flavor == Flavor::Antigravity {
                serde_json::from_str::<Value>(l).map(|ev| agy_events(&ev)).unwrap_or_default()
            } else { backend.parse_line(l) }
        }).collect()
    }

    fn failed_message(events: &[Event]) -> String {
        match events.last() {
            Some(Event::Failed { message }) => message.clone(),
            other => panic!("expected a Failed last, got {other:?}"),
        }
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn req(session: Option<&str>) -> TurnRequest {
        TurnRequest {
            dir: "/tmp".into(),
            model: "m-1".into(),
            prompt: "do the thing".into(),
            images: vec![],
            session_id: session.map(String::from),
            mode: Default::default(),
            trust_project_settings: false,
        }
    }

    #[test]
    fn gemini_turn_parses_to_narration_tool_and_rebuilt_answer() {
        let events = parse_all(&gemini(), GEMINI_TURN);
        // Each written piece comes first, for the live screen; the whole text still follows.
        assert_eq!(events.len(), 7, "{events:?}");
        assert_eq!(events[0], Event::TextDelta("I'll read ".into()));
        assert_eq!(events[1], Event::TextDelta("the file.".into()));
        assert_eq!(events[2], Event::Text("I'll read the file.".into()));
        assert_eq!(events[3], Event::Tool("read_file /work/a.txt".into()));
        assert_eq!(events[4], Event::TextDelta("It says ".into()));
        assert_eq!(events[5], Event::TextDelta("hello.".into()));
        let Event::Finished(f) = &events[6] else { panic!("{:?}", events[6]) };
        // Only the text after the last tool call is the answer.
        assert_eq!(f.text, "It says hello.");
        assert_eq!(f.session_id.as_deref(), Some("6f1c2d3e-4a5b-4c6d-8e7f-0123456789ab"));
        assert_eq!(f.usage, Usage { input_tokens: 1200, cached_input_tokens: 800, output_tokens: 300, context_tokens: None });
    }

    #[test]
    fn a_gemini_message_without_delta_is_not_a_streamed_piece() {
        let events = gemini().parse_line(r#"{"type":"message","role":"assistant","content":"whole"}"#);
        assert_eq!(events, vec![]);
    }

    #[test]
    fn gemini_state_does_not_leak_into_the_next_turn() {
        let b = gemini();
        b.parse_line(r#"{"type":"message","role":"assistant","content":"left over"}"#);
        let events = parse_all(&b, GEMINI_EMPTY_OK);
        assert_eq!(events, vec![Event::Failed { message: EMPTY_ANSWER.into() }]);
    }

    #[test]
    fn gemini_quota_error_carries_the_reset_time() {
        let events = parse_all(&gemini(), GEMINI_QUOTA);
        assert!(matches!(&events[0], Event::Text(t) if t.starts_with("! Attempt 1 failed")));
        let message = failed_message(&events);
        assert_eq!(gemini().classify_error(&message), ErrorKind::Quota { resets_at: Some("3h20m5s".into()) });
    }

    #[test]
    fn gemini_empty_reply_uses_the_earlier_error_event_and_is_empty() {
        let events = parse_all(&gemini(), GEMINI_EMPTY_ERR);
        let message = failed_message(&events);
        assert!(message.contains("empty response"), "{message}");
        assert_eq!(gemini().classify_error(&message), ErrorKind::Empty);
    }

    #[test]
    fn gemini_success_with_no_text_is_failed_not_finished() {
        let events = parse_all(&gemini(), GEMINI_EMPTY_OK);
        assert_eq!(events, vec![Event::Failed { message: EMPTY_ANSWER.into() }]);
        assert_eq!(gemini().classify_error(EMPTY_ANSWER), ErrorKind::Empty);
    }

    #[test]
    fn gemini_exit_41_stderr_is_auth() {
        assert_eq!(gemini().classify_error(GEMINI_AUTH_STDERR.trim()), ErrorKind::Auth);
    }

    #[test]
    fn gemini_failed_tool_result_gives_a_hint() {
        let line = r#"{"type":"tool_result","tool_id":"x","status":"error","error":{"type":"POLICY","message":"denied by policy"}}"#;
        assert_eq!(gemini().parse_line(line), vec![Event::Hint("\u{2717} tool failed: denied by policy".into())]);
        let ok = r#"{"type":"tool_result","tool_id":"x","status":"success","output":"fine"}"#;
        assert!(gemini().parse_line(ok).is_empty());
    }

    #[test]
    fn gemini_error_result_without_any_message_still_fails() {
        let line = r#"{"type":"result","status":"error","stats":{}}"#;
        assert_eq!(gemini().parse_line(line), vec![Event::Failed { message: "gemini reported an error".into() }]);
    }

    #[test]
    fn agy_turn_parses_to_tool_and_finish() {
        let events = parse_all(&agy(), AGY_TURN);
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0], Event::Tool("read_file /work/a.txt".into()));
        let Event::Finished(f) = &events[1] else { panic!("{:?}", events[1]) };
        assert_eq!(f.text, "It says hello.");
        assert_eq!(f.session_id.as_deref(), Some("c0ffee00-1111-2222-3333-444455556666"));
        assert_eq!(f.usage, Usage { input_tokens: 1200, cached_input_tokens: 800, output_tokens: 300, context_tokens: None });
    }

    #[test]
    fn agy_logged_out_envelope_is_auth() {
        let events = parse_all(&agy(), AGY_AUTH);
        let message = failed_message(&events);
        assert_eq!(message, "authentication failed or timed out");
        assert_eq!(agy().classify_error(&message), ErrorKind::Auth);
    }

    #[test]
    fn agy_quota_error_is_quota_with_the_clis_wording() {
        let message = failed_message(&parse_all(&agy(), AGY_QUOTA));
        assert_eq!(agy().classify_error(&message), ErrorKind::Quota { resets_at: Some("in 3 hours".into()) });
    }

    #[test]
    fn agy_success_with_empty_response_is_failed_and_empty() {
        let events = parse_all(&agy(), AGY_EMPTY);
        assert!(events.iter().all(|e| !matches!(e, Event::Finished(_))), "{events:?}");
        let message = failed_message(&events);
        assert_eq!(agy().classify_error(&message), ErrorKind::Empty);
    }

    #[test]
    fn agy_empty_after_denied_actions_says_why() {
        let message = failed_message(&parse_all(&agy(), AGY_EMPTY_DENIED));
        assert!(message.contains("1 action(s) were denied") && message.contains("permissions.allow"), "{message}");
        assert_eq!(agy().classify_error(&message), ErrorKind::Empty);
    }

    #[test]
    fn agy_non_success_statuses_fail() {
        let canceled = r#"{"event":"result","result":{"status":"CANCELED","response":""}}"#;
        assert_eq!(agy_events(&serde_json::from_str::<Value>(canceled).unwrap()), vec![Event::Failed { message: "antigravity ended with status CANCELED".into() }]);
        let waiting = r#"{"event":"result","result":{"response":"partial"}}"#;
        assert!(matches!(&agy_events(&serde_json::from_str::<Value>(waiting).unwrap())[..], [Event::Failed { message }] if message.contains("(none)")));
    }

    #[test]
    fn agy_failed_tool_step_gives_a_hint() {
        let line = r#"{"event":"step_update","step_update":{"step_type":"tool","state":"DONE","tool_info":{"name":"run_command","error":{"type":"DENIED","message":"needs permission"}}}}"#;
        assert_eq!(agy_events(&serde_json::from_str::<Value>(line).unwrap()), vec![Event::Hint("\u{2717} tool failed: needs permission".into())]);
    }

    #[test]
    fn junk_and_unrelated_lines_are_ignored_by_both() {
        for b in [gemini(), agy()] {
            assert!(b.parse_line("Loaded cached credentials.").is_empty());
            assert!(b.parse_line("").is_empty());
            assert!(b.parse_line(r#"{"type":"unknown"}"#).is_empty());
            assert!(b.parse_line(r#"{"event":"unknown"}"#).is_empty());
        }
    }

    #[test]
    fn gemini_command_pipes_the_prompt_and_respects_mode() {
        let b = gemini();
        for (mode, approval) in [(Mode::Ask, "default"), (Mode::Plan, "default"), (Mode::Edit, "auto_edit"), (Mode::Auto, "auto_edit")] {
            let request = TurnRequest { mode, ..req(None) };
            assert_eq!(args(&b.command(&request)), ["-m", "m-1", "--output-format", "stream-json", "--approval-mode", approval]);
            let resumed = TurnRequest { mode, ..req(Some("sid")) };
            assert_eq!(args(&b.command(&resumed)), ["-m", "m-1", "--output-format", "stream-json", "--approval-mode", approval, "--resume", "sid"]);
        }
        assert_eq!(b.stdin_payload(&req(None)).as_deref(), Some("do the thing"));
    }

    #[test]
    fn agy_command_uses_acp_and_keeps_stdin_open_for_approvals() {
        let b = agy();
        assert_eq!(args(&b.command(&req(None))), ["acp"]);
        assert!(b.interactive());
        let init: Value = serde_json::from_str(&b.stdin_payload(&req(None)).unwrap()).unwrap();
        assert_eq!(init["method"], "initialize");
    }

    #[test]
    fn untrusted_workspace_is_unusable_here() {
        let msg = "Gemini CLI is not running in a trusted directory. To proceed, either use `--skip-trust`, set the \
                   `GEMINI_CLI_TRUST_WORKSPACE=true` environment variable, or trust this directory in interactive mode.";
        assert_eq!(gemini().classify_error(msg), ErrorKind::UnusableHere);
        assert_eq!(gemini().classify_error("The workspace is not trusted"), ErrorKind::UnusableHere);
        assert_eq!(agy().classify_error("folder is not trusted"), ErrorKind::UnusableHere);
    }

    #[test]
    fn a_huge_prompt_never_becomes_an_oversized_argument() {
        let prompt = "q".repeat(500_000);
        let request = TurnRequest { prompt: prompt.clone(), ..req(Some("cid")) };
        let b = agy();
        assert_eq!(args(&b.command(&request)), ["acp"]);
        assert!(b.stdin_payload(&request).unwrap().len() < 500);
        let g = gemini();
        assert!(args(&g.command(&request)).iter().all(|a| a.len() < 100));
        assert_eq!(g.stdin_payload(&request).as_deref(), Some(prompt.as_str()));
    }

    #[test]
    fn no_approve_everything_flag_is_ever_passed() {
        let forbidden = ["--yolo", "-y", "yolo", "--dangerously-skip-permissions", "always-proceed"];
        for b in [gemini(), agy()] {
            for session in [None, Some("s")] {
                let all = args(&b.command(&req(session)));
                assert!(all.iter().all(|a| !forbidden.contains(&a.as_str())), "{:?}: {all:?}", b.flavor);
            }
        }
    }

    /// A gemini and an agy turn: fresh, then resumed.
    fn golden_cases(backend: &GeminiBackend) -> String {
        [("fresh", req(None)), ("resumed", req(Some("sess-1")))]
            .map(|(name, r)| format!("=== {name}\n{}\n", serde_json::to_string(&args(&backend.command(&r))).unwrap()))
            .concat()
    }

    #[test]
    fn golden_gemini_args() {
        const MASKED: &str = "# masked: nothing";
        const GOLDEN: &str = golden!("backend/gemini_args.txt");
        let actual = golden_cases(&gemini());
        let (head, body) = GOLDEN.split_once('\n').unwrap();
        assert_eq!(head, MASKED);
        assert_eq!(body, actual);
    }

    #[test]
    fn golden_agy_args() {
        const MASKED: &str = "# masked: nothing";
        const GOLDEN: &str = golden!("backend/agy_args.txt");
        let actual = golden_cases(&agy());
        let (head, body) = GOLDEN.split_once('\n').unwrap();
        assert_eq!(head, MASKED);
        assert_eq!(body, actual);
    }

    #[test]
    fn command_runs_in_the_request_dir_with_the_configured_binary() {
        let b = GeminiBackend::new(Flavor::GeminiCli, "/opt/gemini");
        let cmd = b.command(&req(None));
        assert_eq!(cmd.as_std().get_program(), "/opt/gemini");
        assert_eq!(cmd.as_std().get_current_dir(), Some(std::path::Path::new("/tmp")));
    }

    #[test]
    fn extra_env_reaches_the_child_so_a_key_kept_in_the_env_file_works() {
        let b = GeminiBackend::new(Flavor::GeminiCli, "gemini").with_env(vec![("GEMINI_API_KEY".into(), "k".into())]);
        let cmd = b.command(&req(None));
        assert!(cmd.as_std().get_envs().any(|(k, v)| k == "GEMINI_API_KEY" && v == Some(std::ffi::OsStr::new("k"))));
    }

    #[test]
    fn both_flavors_are_the_gemini_provider() {
        assert_eq!(gemini().provider(), ProviderId::Gemini);
        assert_eq!(agy().provider(), ProviderId::Gemini);
        assert_eq!(GeminiBackend::gemini_cli().flavor, Flavor::GeminiCli);
        assert_eq!(GeminiBackend::antigravity().flavor, Flavor::Antigravity);
    }

    #[test]
    fn binary_falls_back_to_the_default_name() {
        assert_eq!(bin_from_env("JEV_TEST_UNSET_BIN_VAR", "gemini"), "gemini");
    }

    #[test]
    fn classifies_other_errors_and_common_ones() {
        assert_eq!(gemini().classify_error("segmentation fault"), ErrorKind::Other);
        assert!(matches!(gemini().classify_error("You have hit your usage limit"), ErrorKind::Quota { .. }));
        assert_eq!(gemini().classify_error("RESOURCE_EXHAUSTED"), ErrorKind::Quota { resets_at: None });
        assert_eq!(reset_after("Your quota will reset after 59s"), Some("59s".into()));
        assert_eq!(reset_after("no hint here"), None);
    }

    #[test]
    fn auth_is_configured_by_env_or_a_named_auth_type() {
        assert!(auth_looks_configured(None, true));
        assert!(auth_looks_configured(Some("{}"), true));
        assert!(!auth_looks_configured(None, false));
        assert!(!auth_looks_configured(Some("{}"), false));
        assert!(auth_looks_configured(Some(r#"{"security":{"auth":{"selectedType":"oauth-personal"}}}"#), false));
        assert!(auth_looks_configured(Some(r#"{"selectedAuthType":"gemini-api-key"}"#), false));
    }

    #[test]
    fn auth_settings_edge_cases() {
        assert!(!auth_looks_configured(Some(r#"{"security":{"auth":{"selectedType":"  "}}}"#), false));
        assert!(!auth_looks_configured(Some(r#"{"security":{"auth":{}}}"#), false));
        assert!(!auth_looks_configured(Some("not json"), false));
        assert!(!auth_looks_configured(Some(""), false));
        let commented = "{\n  // how to log in\n  \"security\": {\"auth\": {\"selectedType\": \"vertex-ai\"}}\n}";
        assert!(auth_looks_configured(Some(commented), false));
    }

    #[test]
    fn is_configured_is_the_one_rule_for_settings_and_environment() {
        let commented = "{\n  // how to log in\n  \"security\": {\"auth\": {\"selectedType\": \"vertex-ai\"}}\n}";
        let none = |_: &str| false;
        assert!(is_configured(Some(commented), none), "a commented settings.json that names an auth type");
        for var in AUTH_ENV_VARS {
            assert!(is_configured(None, |n| n == var), "{var} alone is enough");
        }
        assert!(is_configured(None, |n| n == "GOOGLE_GENAI_USE_VERTEXAI"), "Vertex env only");
        assert!(!is_configured(None, none));
        assert!(!is_configured(Some("{}"), none));
        assert!(!is_configured(Some(r#"{"theme":"dark"}"#), |n| n == "SOMETHING_ELSE"));
    }

    #[test]
    fn auth_wording_must_say_the_login_failed_not_just_mention_authentication() {
        let b = GeminiBackend::gemini_cli();
        assert_eq!(b.classify_error("authenticate the user in the request handler"), ErrorKind::Other);
        assert_eq!(b.classify_error("the sign in page redirects twice"), ErrorKind::Other);
        assert_eq!(b.classify_error("Authentication failed: please sign in again"), ErrorKind::Auth);
    }
}
