//! Which providers the user can actually use right now. Decided once at startup,
//! BEFORE the first turn, because finding out by running a turn is slow (a logged-out
//! `codex` retries for ~35 s before it fails) and a turn sent to a provider the user
//! cannot use is a wasted turn.
//!
//! Every provider has the same shape: a thin IO function gathers cheap facts (a PATH
//! search, a status command, a small file), and a pure `decide_*` function turns
//! those facts into an [`Availability`]. Only the pure half is unit-tested, with
//! injected inputs, so no test touches real home files, real CLIs or the network.
//!
//! No probe ever sends a prompt. `grok` is never executed at all: here it is a lazy
//! mise wrapper and even `--version` starts a long install.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;

use crate::backend::{gemini, grok, ProviderId};
use crate::envfile;

/// Each probe gets this long; a slower one is reported as [`State::Unknown`] rather
/// than holding up startup.
const PROBE_CAP: Duration = Duration::from_secs(5);

/// Can the user use this provider?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ready,
    NotInstalled,
    NotLoggedIn,
    /// An API-key provider with no key configured.
    NoKey,
    /// The probe could not tell (timed out, crashed, unreadable output).
    Unknown,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotInstalled => "not installed",
            Self::NotLoggedIn => "not logged in",
            Self::NoKey => "no key",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Availability {
    pub provider: ProviderId,
    pub state: State,
    /// The account plan when the login says so ("max_5x", "plus", "pay-per-token").
    pub plan: Option<String>,
    /// Models this login may use, when the CLI lists them. `None` means "no list known".
    pub models: Option<Vec<String>>,
    /// Why, in a few words, for `/providers`.
    pub detail: String,
    /// False when the probe was written from documentation and has never been
    /// checked against the real CLI.
    pub verified: bool,
}

/// Whether a provider's probe has been run against the real CLI. One place, so a
/// probe is promoted to verified by changing one match arm.
fn is_verified(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::Claude | ProviderId::Codex | ProviderId::DeepSeek)
}

fn avail(provider: ProviderId, state: State, detail: impl Into<String>) -> Availability {
    Availability { provider, state, plan: None, models: None, detail: detail.into(), verified: is_verified(provider) }
}

// ---------------------------------------------------------------- public API

/// Probe all five providers concurrently, each capped at [`PROBE_CAP`].
pub async fn probe_all() -> Vec<Availability> {
    let (claude, deepseek, codex, gemini, grok) = tokio::join!(
        capped(PROBE_CAP, ProviderId::Claude, probe_claude()),
        capped(PROBE_CAP, ProviderId::DeepSeek, probe_deepseek()),
        capped(PROBE_CAP, ProviderId::Codex, probe_codex()),
        capped(PROBE_CAP, ProviderId::Gemini, probe_gemini()),
        capped(PROBE_CAP, ProviderId::Grok, probe_grok()),
    );
    vec![claude, deepseek, codex, gemini, grok]
}

/// The (provider, model) pairs a turn may be routed to: for each Ready provider, the
/// models its login lists, else everything `all_models` knows for it. Providers that
/// are not Ready contribute nothing, so routing can never pick one. The catalog is
/// injected so this stays pure.
pub fn usable_models(avail: &[Availability], all_models: &dyn Fn(ProviderId) -> Vec<String>) -> Vec<(ProviderId, String)> {
    avail
        .iter()
        .filter(|a| a.state == State::Ready)
        .flat_map(|a| {
            let models = a.models.clone().unwrap_or_else(|| all_models(a.provider));
            models.into_iter().map(move |m| (a.provider, m))
        })
        .collect()
}

/// The lines `/providers` prints, one per provider.
pub fn render(avail: &[Availability]) -> String {
    avail.iter().map(render_line).collect::<Vec<_>>().join("\n")
}

/// Only the providers that are ready, for the compact startup summary.
pub fn render_ready(avail: &[Availability]) -> String {
    avail.iter().filter(|a| a.state == State::Ready).map(render_line).collect::<Vec<_>>().join("\n")
}

fn render_line(a: &Availability) -> String {
    let plan = a.plan.as_deref().unwrap_or("-");
    let models = match (&a.state, &a.models) {
        (State::Ready, Some(list)) => list.join(", "),
        (State::Ready, None) => "all".to_string(),
        _ => "-".to_string(),
    };
    let unverified = if a.verified { "" } else { " (unverified)" };
    format!(
        "{:<9} {:<13} plan: {}  models: {}  {}{}",
        a.provider.as_str(),
        a.state.label(),
        plan,
        models,
        a.detail,
        unverified
    )
}

// ------------------------------------------------------------ time cap (pure-ish)

/// Run one probe under `cap`. A probe that hangs must not delay startup, and "could
/// not tell" is safe: Unknown is not Ready, so nothing is routed there.
async fn capped(cap: Duration, provider: ProviderId, probe: impl Future<Output = Availability>) -> Availability {
    match tokio::time::timeout(cap, probe).await {
        Ok(a) => a,
        Err(_) => avail(provider, State::Unknown, format!("probe timed out after {}s", cap.as_secs_f32())),
    }
}

// ------------------------------------------------------------------ PATH search

/// Every executable called `name` in `dirs`, in PATH order. `is_exec` is injected so
/// the search is testable without a filesystem.
fn find_on_path(name: &str, dirs: &[PathBuf], is_exec: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    dirs.iter().map(|d| d.join(name)).filter(|p| is_exec(p)).collect()
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default()
}

/// Installed means "found on PATH"; the binary is not run, since some are lazy
/// wrappers whose first run starts an install.
pub(crate) fn on_path(name: &str) -> bool {
    if name.contains('/') {
        return is_executable_file(Path::new(name));
    }
    !find_on_path(name, &path_dirs(), &is_executable_file).is_empty()
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn read_home_file(rel: &str) -> Option<String> {
    std::fs::read_to_string(home()?.join(rel)).ok()
}

fn home_path_exists(rel: &str) -> bool {
    home().is_some_and(|h| h.join(rel).exists())
}

// ---------------------------------------------------------------- run a status CLI

/// What a short status command printed. `text` is stdout then stderr, because CLIs
/// disagree on which stream carries the answer.
struct Captured {
    ok: bool,
    text: String,
}

/// Run `program args` with no stdin, and wait for it. `None` when it cannot start.
/// The child is killed if the probe is cancelled by the time cap.
async fn run_capture(program: &str, args: &[&str]) -> Option<Captured> {
    let out = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Some(Captured { ok: out.status.success(), text })
}

// ------------------------------------------------------------------------ Claude

/// What `claude auth status` said.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaudeLogin {
    logged_in: bool,
    plan: Option<String>,
}

/// `claude auth status` prints JSON by default (`loggedIn`, `planTier`,
/// `subscriptionType`); the text form is read too so a changed default does not
/// break the probe. `None` when neither says anything clear.
fn parse_claude_login(text: &str) -> Option<ClaudeLogin> {
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        let logged_in = v.get("loggedIn")?.as_bool()?;
        let plan = ["planTier", "subscriptionType"]
            .iter()
            .find_map(|k| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()))
            .map(str::to_string);
        return Some(ClaudeLogin { logged_in, plan });
    }
    let lower = text.to_lowercase();
    if lower.contains("not logged in") || lower.contains("not authenticated") {
        Some(ClaudeLogin { logged_in: false, plan: None })
    } else if lower.contains("logged in") {
        Some(ClaudeLogin { logged_in: true, plan: None })
    } else {
        None
    }
}

fn decide_claude(installed: bool, login: Option<ClaudeLogin>) -> Availability {
    let id = ProviderId::Claude;
    if !installed {
        return avail(id, State::NotInstalled, "`claude` not found on PATH");
    }
    match login {
        None => avail(id, State::Unknown, "could not read `claude auth status`"),
        Some(l) if !l.logged_in => avail(id, State::NotLoggedIn, "run `claude auth login`"),
        Some(l) => Availability { plan: l.plan, ..avail(id, State::Ready, "logged in") },
    }
}

async fn probe_claude() -> Availability {
    let installed = on_path("claude");
    let login = match installed {
        true => run_capture("claude", &["auth", "status"]).await.and_then(|c| parse_claude_login(&c.text)),
        false => None,
    };
    decide_claude(installed, login)
}

// ------------------------------------------------------------------------ DeepSeek

/// DeepSeek is the `claude` CLI pointed at another API, so it needs both the binary
/// and a key.
fn decide_deepseek(claude_installed: bool, key_present: bool) -> Availability {
    let id = ProviderId::DeepSeek;
    if !claude_installed {
        return avail(id, State::NotInstalled, "needs the `claude` CLI, not found on PATH");
    }
    if !key_present {
        return avail(id, State::NoKey, "set DEEPSEEK_API_KEY (environment or ~/.jev-router.env)");
    }
    Availability { plan: Some("pay-per-token".into()), ..avail(id, State::Ready, "DEEPSEEK_API_KEY set") }
}

async fn probe_deepseek() -> Availability {
    decide_deepseek(on_path("claude"), envfile::lookup("DEEPSEEK_API_KEY").is_some())
}

// ------------------------------------------------------------------------ Codex

/// `codex login status`: `Some(true/false)` when its output is clear. A non-zero
/// exit means not logged in; "Not logged in" is checked first because it contains
/// "logged in".
fn parse_codex_login(ok: bool, text: &str) -> Option<bool> {
    let lower = text.to_lowercase();
    if lower.contains("not logged in") || !ok {
        Some(false)
    } else if lower.contains("logged in") {
        Some(true)
    } else {
        None
    }
}

fn decode_base64url(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '-' | '+' => 62,
            '_' | '/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The ChatGPT plan ("free", "plus", ...) from `~/.codex/auth.json`: the
/// `chatgpt_plan_type` claim inside the `id_token` JWT. The token is only read, not
/// verified: the plan is shown to the user, never trusted for a decision.
fn codex_plan_from_auth(auth_json: &str) -> Option<String> {
    let auth: Value = serde_json::from_str(auth_json).ok()?;
    let payload = auth.pointer("/tokens/id_token")?.as_str()?.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(&decode_base64url(payload)?).ok()?;
    let plan = claims.pointer("/https:~1~1api.openai.com~1auth/chatgpt_plan_type")?.as_str()?;
    (!plan.is_empty()).then(|| plan.to_string())
}

/// `login_ok` is `None` when `codex login status` could not be run or read.
fn decide_codex(installed: bool, login_ok: Option<bool>, plan: Option<&str>, models: Option<Vec<String>>) -> Availability {
    let id = ProviderId::Codex;
    if !installed {
        return avail(id, State::NotInstalled, "`codex` not found on PATH");
    }
    match login_ok {
        None => avail(id, State::Unknown, "could not read `codex login status`"),
        Some(false) => avail(id, State::NotLoggedIn, "run `codex login`"),
        Some(true) => Availability {
            plan: plan.map(str::to_string),
            models: models.filter(|m| !m.is_empty()),
            ..avail(id, State::Ready, "logged in with ChatGPT")
        },
    }
}

async fn probe_codex() -> Availability {
    let installed = on_path("codex");
    if !installed {
        return decide_codex(false, None, None, None);
    }
    let login_ok = run_capture("codex", &["login", "status"]).await.and_then(|c| parse_codex_login(c.ok, &c.text));
    let plan = read_home_file(".codex/auth.json").and_then(|t| codex_plan_from_auth(&t));
    let models = read_home_file(".codex/models_cache.json").map(|t| crate::backend::codex::allowed_models_from_cache(&t));
    decide_codex(installed, login_ok, plan.as_deref(), models)
}

// ------------------------------------------------------------------------ Gemini

/// Everything the Gemini decision needs. Two CLIs serve it (`gemini` and `agy`), and
/// each has several ways of being signed in, so the facts are listed rather than
/// computed in the decision.
#[derive(Debug, Clone, Copy, Default)]
struct GeminiFacts {
    gemini_installed: bool,
    agy_installed: bool,
    /// An API-key or Vertex/Code Assist variable in the environment (`gemini::AUTH_ENV_VARS`).
    api_key_env: bool,
    /// `~/.gemini/settings.json` names an auth type.
    settings_auth: bool,
    /// Cached OAuth login, `~/.gemini/oauth_creds.json`.
    oauth_creds: bool,
    /// The agy login directory `~/.gemini/antigravity-cli/` has files in it.
    agy_creds: bool,
}

/// The two Gemini facts that come from the settings file and the environment,
/// both decided by [`gemini::is_configured`], the rule the provider factory uses
/// too. Returns `(api_key_env, settings_auth)`.
fn gemini_login_facts(settings: Option<&str>, env_set: impl Fn(&str) -> bool) -> (bool, bool) {
    (gemini::is_configured(None, env_set), gemini::is_configured(settings, |_| false))
}

/// Which sign-in was found, for the detail line.
fn gemini_auth_source(f: &GeminiFacts) -> Option<&'static str> {
    [
        (f.api_key_env, "API key in environment"),
        (f.settings_auth, "auth type set in ~/.gemini/settings.json"),
        (f.oauth_creds, "cached OAuth login"),
        (f.agy_creds, "agy login"),
    ]
    .into_iter()
    .find_map(|(present, what)| present.then_some(what))
}

fn decide_gemini(f: GeminiFacts) -> Availability {
    let id = ProviderId::Gemini;
    if !f.gemini_installed && !f.agy_installed {
        return avail(id, State::NotInstalled, "neither `gemini` nor `agy` found on PATH");
    }
    match gemini_auth_source(&f) {
        Some(source) => avail(id, State::Ready, source),
        None => avail(id, State::NotLoggedIn, "no API key, settings auth type or login found (`gemini` exits 41 without auth)"),
    }
}

fn dir_has_entries(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut d| d.next().is_some())
}

async fn probe_gemini() -> Availability {
    let settings = read_home_file(".gemini/settings.json");
    let (api_key_env, settings_auth) = gemini_login_facts(settings.as_deref(), |n| envfile::lookup(n).is_some());
    decide_gemini(GeminiFacts {
        gemini_installed: on_path("gemini"),
        agy_installed: on_path("agy"),
        api_key_env,
        settings_auth,
        oauth_creds: home_path_exists(".gemini/oauth_creds.json"),
        agy_creds: home().is_some_and(|h| dir_has_entries(&h.join(".gemini/antigravity-cli"))),
    })
}

// ------------------------------------------------------------------------ Grok

/// Grok is decided from the filesystem alone (see the module docs).
#[derive(Debug, Clone, Copy, Default)]
struct GrokFacts {
    /// A real binary exists, not just the lazy mise command-wrapper.
    real_binary: bool,
    /// `~/.grok` exists: the CLI has been signed in or used.
    home_dir: bool,
}

fn decide_grok(f: GrokFacts) -> Availability {
    let id = ProviderId::Grok;
    match (f.real_binary, f.home_dir) {
        (false, _) => avail(id, State::NotInstalled, "no installed `grok` (only a lazy mise wrapper, if anything)"),
        (true, false) => avail(id, State::NotLoggedIn, "installed, but ~/.grok is missing: sign in first"),
        (true, true) => avail(id, State::Ready, "installed, ~/.grok present"),
    }
}

/// A command-wrapper is the lazy launcher that installs the tool on first run.
fn is_mise_wrapper(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "command-wrappers")
}

/// mise's data directory: `$MISE_DATA_DIR`, else `$XDG_DATA_HOME/mise`, else
/// `~/.local/share/mise`.
fn mise_data_dir(mise_data_dir: Option<String>, xdg_data_home: Option<String>, home: Option<PathBuf>) -> Option<PathBuf> {
    mise_data_dir
        .map(PathBuf::from)
        .or_else(|| xdg_data_home.map(|x| PathBuf::from(x).join("mise")))
        .or_else(|| home.map(|h| h.join(".local/share/mise")))
}

/// True when some version directory under `<data>/installs/grok/` has files in it.
fn mise_has_grok_install(data_dir: &Path) -> bool {
    std::fs::read_dir(data_dir.join("installs/grok"))
        .map(|versions| versions.flatten().any(|v| dir_has_entries(&v.path())))
        .unwrap_or(false)
}

async fn probe_grok() -> Availability {
    let data = mise_data_dir(std::env::var("MISE_DATA_DIR").ok(), std::env::var("XDG_DATA_HOME").ok(), home());
    let on_path_real = find_on_path("grok", &path_dirs(), &is_executable_file).iter().any(|p| !is_mise_wrapper(p));
    decide_grok(GrokFacts {
        real_binary: on_path_real || data.is_some_and(|d| mise_has_grok_install(&d)),
        home_dir: home().is_some_and(|h| grok::session_dir_exists(&h)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(p: ProviderId, models: Option<Vec<&str>>) -> Availability {
        Availability {
            models: models.map(|m| m.into_iter().map(String::from).collect()),
            ..avail(p, State::Ready, "ok")
        }
    }

    // ---- Claude

    #[test]
    fn claude_json_status_logged_in_with_plan() {
        let l = parse_claude_login(r#"{"loggedIn":true,"subscriptionType":"max","planTier":"max_5x"}"#).unwrap();
        assert_eq!(l, ClaudeLogin { logged_in: true, plan: Some("max_5x".into()) });
        let l = parse_claude_login(r#"{"loggedIn":true,"subscriptionType":"pro"}"#).unwrap();
        assert_eq!(l.plan.as_deref(), Some("pro"));
    }

    #[test]
    fn claude_json_status_logged_out() {
        let l = parse_claude_login("{\n  \"loggedIn\": false\n}\n").unwrap();
        assert!(!l.logged_in);
    }

    #[test]
    fn claude_text_status_is_understood() {
        assert!(!parse_claude_login("Not logged in").unwrap().logged_in);
        assert!(parse_claude_login("Logged in as someone").unwrap().logged_in);
        assert_eq!(parse_claude_login("something else"), None);
        assert_eq!(parse_claude_login("{\"other\":1}"), None);
    }

    #[test]
    fn claude_decisions() {
        let login = |logged_in, plan: Option<&str>| Some(ClaudeLogin { logged_in, plan: plan.map(String::from) });
        assert_eq!(decide_claude(false, login(true, None)).state, State::NotInstalled);
        assert_eq!(decide_claude(true, None).state, State::Unknown);
        assert_eq!(decide_claude(true, login(false, None)).state, State::NotLoggedIn);
        let a = decide_claude(true, login(true, Some("max_5x")));
        assert_eq!((a.state, a.plan.as_deref(), a.verified), (State::Ready, Some("max_5x"), true));
    }

    // ---- DeepSeek

    #[test]
    fn deepseek_decisions() {
        assert_eq!(decide_deepseek(false, true).state, State::NotInstalled);
        assert_eq!(decide_deepseek(true, false).state, State::NoKey);
        let a = decide_deepseek(true, true);
        assert_eq!((a.state, a.plan.as_deref()), (State::Ready, Some("pay-per-token")));
    }

    // ---- Codex

    #[test]
    fn codex_login_text() {
        assert_eq!(parse_codex_login(true, "Logged in using ChatGPT\n"), Some(true));
        assert_eq!(parse_codex_login(false, "Not logged in"), Some(false));
        assert_eq!(parse_codex_login(true, "Not logged in"), Some(false));
        assert_eq!(parse_codex_login(false, ""), Some(false));
        assert_eq!(parse_codex_login(true, "???"), None);
    }

    #[test]
    fn base64url_decoding() {
        assert_eq!(decode_base64url("aGVsbG8").unwrap(), b"hello");
        assert_eq!(decode_base64url("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64url("Pz8_Pz8-").unwrap(), b"?????>");
        assert_eq!(decode_base64url("a$b"), None);
        assert_eq!(decode_base64url("").unwrap(), b"");
    }

    /// Builds a fake auth.json whose JWT payload is `claims` (synthetic, never real).
    fn fake_auth(claims: &str) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut payload = String::new();
        for chunk in claims.as_bytes().chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |a, (i, b)| a | (*b as u32) << (16 - 8 * i));
            let chars = chunk.len() + 1;
            for i in 0..chars {
                payload.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        format!(r#"{{"tokens":{{"id_token":"head.{payload}.sig"}}}}"#)
    }

    #[test]
    fn codex_plan_comes_from_the_jwt_claim() {
        let auth = fake_auth(r#"{"https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}"#);
        assert_eq!(codex_plan_from_auth(&auth).as_deref(), Some("plus"));
    }

    #[test]
    fn codex_plan_is_none_when_absent_or_malformed() {
        assert_eq!(codex_plan_from_auth(&fake_auth(r#"{"sub":"x"}"#)), None);
        assert_eq!(codex_plan_from_auth(r#"{"tokens":{"id_token":"nodots"}}"#), None);
        assert_eq!(codex_plan_from_auth(r#"{"tokens":{}}"#), None);
        assert_eq!(codex_plan_from_auth("not json"), None);
    }


    #[test]
    fn codex_decisions() {
        let models = || Some(vec!["m1".to_string()]);
        assert_eq!(decide_codex(false, Some(true), None, None).state, State::NotInstalled);
        assert_eq!(decide_codex(true, None, None, None).state, State::Unknown);
        assert_eq!(decide_codex(true, Some(false), Some("plus"), models()).state, State::NotLoggedIn);
        let a = decide_codex(true, Some(true), Some("free"), models());
        assert_eq!((a.state, a.plan.as_deref(), a.models), (State::Ready, Some("free"), models()));
        assert_eq!(decide_codex(true, Some(true), None, Some(vec![])).models, None);
    }

    // ---- Gemini

    #[test]
    fn gemini_login_facts_follow_the_factory_rule() {
        let commented = "{\n  // login\n  \"security\": {\"auth\": {\"selectedType\": \"oauth-personal\"}}\n}";
        let no_env = |_: &str| false;
        assert_eq!(gemini_login_facts(Some(commented), no_env), (false, true), "comments in settings.json are fine");
        assert_eq!(gemini_login_facts(None, |n| n == "GOOGLE_GENAI_USE_VERTEXAI"), (true, false), "Vertex env only");
        assert_eq!(gemini_login_facts(None, |n| n == "GOOGLE_GENAI_USE_GCA"), (true, false));
        assert_eq!(gemini_login_facts(Some(r#"{"theme":"dark"}"#), no_env), (false, false));
        assert_eq!(gemini_login_facts(Some("nope"), no_env), (false, false));
        assert_eq!(gemini_login_facts(None, no_env), (false, false));
    }

    #[test]
    fn gemini_decisions() {
        let installed = GeminiFacts { gemini_installed: true, ..Default::default() };
        assert_eq!(decide_gemini(GeminiFacts::default()).state, State::NotInstalled);
        assert_eq!(decide_gemini(installed).state, State::NotLoggedIn);
        for configured in [
            GeminiFacts { api_key_env: true, ..installed },
            GeminiFacts { settings_auth: true, ..installed },
            GeminiFacts { oauth_creds: true, ..installed },
            GeminiFacts { agy_installed: true, gemini_installed: false, agy_creds: true, ..Default::default() },
        ] {
            let a = decide_gemini(configured);
            assert_eq!((a.state, a.verified), (State::Ready, false));
        }
        // A login with no CLI to use it is not usable.
        assert_eq!(decide_gemini(GeminiFacts { oauth_creds: true, ..Default::default() }).state, State::NotInstalled);
    }

    // ---- Grok

    #[test]
    fn grok_decisions() {
        assert_eq!(decide_grok(GrokFacts::default()).state, State::NotInstalled);
        assert_eq!(decide_grok(GrokFacts { real_binary: false, home_dir: true }).state, State::NotInstalled);
        assert_eq!(decide_grok(GrokFacts { real_binary: true, home_dir: false }).state, State::NotLoggedIn);
        let a = decide_grok(GrokFacts { real_binary: true, home_dir: true });
        assert_eq!((a.state, a.verified), (State::Ready, false));
    }

    #[test]
    fn mise_wrapper_paths_are_recognised() {
        assert!(is_mise_wrapper(Path::new("/h/.local/share/mise/command-wrappers/bin/grok")));
        assert!(!is_mise_wrapper(Path::new("/h/.local/share/mise/installs/grok/1.0/bin/grok")));
        assert!(!is_mise_wrapper(Path::new("/usr/bin/grok")));
    }

    #[test]
    fn mise_data_dir_precedence() {
        let h = Some(PathBuf::from("/h"));
        assert_eq!(mise_data_dir(Some("/m".into()), Some("/x".into()), h.clone()), Some(PathBuf::from("/m")));
        assert_eq!(mise_data_dir(None, Some("/x".into()), h.clone()), Some(PathBuf::from("/x/mise")));
        assert_eq!(mise_data_dir(None, None, h), Some(PathBuf::from("/h/.local/share/mise")));
        assert_eq!(mise_data_dir(None, None, None), None);
    }

    // ---- PATH search

    #[test]
    fn find_on_path_returns_matches_in_order() {
        let dirs = [PathBuf::from("/a"), PathBuf::from("/b"), PathBuf::from("/c")];
        let exists = |p: &Path| p == Path::new("/b/tool") || p == Path::new("/c/tool");
        assert_eq!(find_on_path("tool", &dirs, &exists), vec![PathBuf::from("/b/tool"), PathBuf::from("/c/tool")]);
        assert!(find_on_path("other", &dirs, &exists).is_empty());
    }

    // ---- usable_models

    fn catalog(p: ProviderId) -> Vec<String> {
        match p {
            ProviderId::Claude => vec!["opus".into(), "sonnet".into()],
            ProviderId::Codex => vec!["codex-all".into()],
            _ => vec![],
        }
    }

    #[test]
    fn usable_models_uses_listed_models_else_the_catalog_and_skips_unready() {
        let list = vec![
            ready(ProviderId::Claude, None),
            ready(ProviderId::Codex, Some(vec!["gpt-a"])),
            avail(ProviderId::Gemini, State::NotLoggedIn, "x"),
            avail(ProviderId::DeepSeek, State::Unknown, "x"),
        ];
        assert_eq!(
            usable_models(&list, &catalog),
            vec![
                (ProviderId::Claude, "opus".to_string()),
                (ProviderId::Claude, "sonnet".to_string()),
                (ProviderId::Codex, "gpt-a".to_string()),
            ]
        );
        assert!(usable_models(&[], &catalog).is_empty());
    }

    // ---- render

    #[test]
    fn render_shows_state_plan_models_detail_and_unverified() {
        let list = vec![
            Availability { plan: Some("max_5x".into()), ..ready(ProviderId::Claude, None) },
            ready(ProviderId::Codex, Some(vec!["a", "b"])),
            Availability { verified: false, ..avail(ProviderId::Grok, State::NotLoggedIn, "sign in") },
        ];
        let out = render(&list);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("claude") && lines[0].contains("ready") && lines[0].contains("plan: max_5x"));
        assert!(lines[0].contains("models: all") && !lines[0].contains("unverified"));
        assert!(lines[1].contains("models: a, b") && lines[1].contains("plan: -"));
        assert!(lines[2].contains("not logged in") && lines[2].contains("models: -") && lines[2].ends_with("(unverified)"));
    }

    // ---- time cap

    #[tokio::test]
    async fn slow_probe_becomes_unknown() {
        let never = std::future::pending::<Availability>();
        let a = capped(Duration::from_millis(30), ProviderId::Grok, never).await;
        assert_eq!((a.provider, a.state), (ProviderId::Grok, State::Unknown));
        assert!(a.detail.contains("timed out") && !a.verified);
    }

    #[tokio::test]
    async fn fast_probe_passes_through() {
        let a = capped(Duration::from_secs(1), ProviderId::Claude, async { ready(ProviderId::Claude, None) }).await;
        assert_eq!(a.state, State::Ready);
    }
}
