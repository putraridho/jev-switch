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

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::infrastructure::gemini::gemini_cli::auth as gemini;
use crate::backend::infrastructure::grok::grok_cli as grok;
use crate::backend::ProviderId;
use crate::shared::infrastructure::env_file as envfile;

/// Each probe gets this long; a slower one is reported as [`State::Unknown`] rather
/// than holding up startup.
pub const PROBE_CAP: Duration = Duration::from_secs(5);

/// Can the user use this provider?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
        capped(PROBE_CAP, ProviderId::Claude, probe_provider(ProviderId::Claude)),
        capped(PROBE_CAP, ProviderId::DeepSeek, probe_provider(ProviderId::DeepSeek)),
        capped(PROBE_CAP, ProviderId::Codex, probe_provider(ProviderId::Codex)),
        capped(PROBE_CAP, ProviderId::Gemini, probe_provider(ProviderId::Gemini)),
        capped(PROBE_CAP, ProviderId::Grok, probe_provider(ProviderId::Grok)),
    );
    vec![claude, deepseek, codex, gemini, grok]
}

/// One provider's probe, with no time cap of its own: the caller says how long it may take.
pub async fn probe_provider(provider: ProviderId) -> Availability {
    match provider {
        ProviderId::Claude => probe_claude().await,
        ProviderId::DeepSeek => probe_deepseek().await,
        ProviderId::Codex => probe_codex().await,
        ProviderId::Gemini => probe_gemini().await,
        ProviderId::Grok => probe_grok().await,
    }
}

// ------------------------------------------------------------- the slow-probe retry

/// A probe that ended [`State::Unknown`] is tried once more, in the background, with this
/// longer cap. `codex` and `claude` are often lazy wrappers (mise) whose first run can take
/// longer than [`PROBE_CAP`] while they install or warm up; waiting that long at startup
/// would stall the banner, and never asking again would leave the provider unused all session.
pub const RETRY_CAP: Duration = Duration::from_secs(20);

/// The providers whose first probe could not tell, and so deserve the retry.
pub fn retry_candidates(first: &[Availability]) -> Vec<ProviderId> {
    first.iter().filter(|a| a.state == State::Unknown).map(|a| a.provider).collect()
}

/// Probe each of `providers` once more, all at once, each under `cap`, and hand every
/// answer to `done` as it arrives (so a quick one is not held back by a slow one). It is
/// the whole policy: one retry, never a loop. `probe` is injected so tests need no CLI.
pub async fn retry_unknown<P, Fut>(providers: Vec<ProviderId>, cap: Duration, probe: P, mut done: impl FnMut(Availability))
where
    P: Fn(ProviderId) -> Fut,
    Fut: Future<Output = Availability> + Send + 'static,
{
    let mut running = tokio::task::JoinSet::new();
    for provider in providers {
        running.spawn(capped(cap, provider, probe(provider)));
    }
    while let Some(finished) = running.join_next().await {
        if let Ok(answer) = finished {
            done(answer);
        }
    }
}

/// Fold a late answer into `current`: it only fills in a provider that is still
/// [`State::Unknown`] (a `/providers refresh` may have found a definite answer since), and an
/// answer that is itself Unknown teaches nothing. True when something changed.
pub fn merge_late(current: &mut [Availability], late: Availability) -> bool {
    if late.state == State::Unknown {
        return false;
    }
    match current.iter_mut().find(|a| a.provider == late.provider && a.state == State::Unknown) {
        Some(slot) => {
            *slot = late;
            true
        }
        None => false,
    }
}

/// Fold a fresh answer into `current` (which came from the cache): it replaces the provider's
/// entry when it says something different. An Unknown answer teaches nothing and is dropped.
/// True when something changed.
pub fn merge_refresh(current: &mut [Availability], fresh: Availability) -> bool {
    if fresh.state == State::Unknown {
        return false;
    }
    match current.iter_mut().find(|a| a.provider == fresh.provider) {
        Some(slot) if *slot != fresh => {
            *slot = fresh;
            true
        }
        _ => false,
    }
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
/// A status probe's command. Probes run in mise's offline mode: on Omarchy the agent CLIs
/// are mise launchers that update themselves before they run (at most every 4 hours), and a
/// probe that triggers that update blows its 5 s cap (the `? codex` at startup) and can make
/// mise install other tools. Offline, the installed version answers in well under a second
/// (measured 0.4 s for `codex login status`, 0.6 s for `claude auth status`) and nothing is
/// fetched. Real turns are NOT run offline: they behave exactly as when the user runs the tool.
fn probe_command(program: &str, args: &[&str]) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).env("MISE_OFFLINE", "1").stdin(Stdio::null()).kill_on_drop(true);
    cmd
}

async fn run_capture(program: &str, args: &[&str]) -> Option<Captured> {
    let out = probe_command(program, args).output().await.ok()?;
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
    let installed = installed_for_real("claude");
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
    decide_deepseek(installed_for_real("claude"), envfile::lookup("DEEPSEEK_API_KEY").is_some())
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
    let installed = installed_for_real("codex");
    if !installed {
        return decide_codex(false, None, None, None);
    }
    let login_ok = run_capture("codex", &["login", "status"]).await.and_then(|c| parse_codex_login(c.ok, &c.text));
    let plan = read_home_file(".codex/auth.json").and_then(|t| codex_plan_from_auth(&t));
    let models = read_home_file(".codex/models_cache.json").map(|t| crate::backend::infrastructure::codex::codex_cli::models::allowed_models_from_cache(&t));
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
        gemini_installed: installed_for_real("gemini"),
        agy_installed: installed_for_real("agy"),
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
    /// `XAI_API_KEY` is set in the environment or `~/.jev-router.env`.
    api_key: bool,
}

fn decide_grok(f: GrokFacts) -> Availability {
    let id = ProviderId::Grok;
    match (f.real_binary, f.api_key, f.home_dir) {
        (false, _, _) => avail(id, State::NotInstalled, "no installed `grok` (only a lazy mise wrapper, if anything)"),
        (true, true, _) => avail(id, State::Ready, "installed, XAI_API_KEY set"),
        (true, false, true) => avail(id, State::Ready, "installed, ~/.grok present"),
        (true, false, false) => avail(id, State::NotLoggedIn, "installed, but no ~/.grok or XAI_API_KEY: sign in or set the key"),
    }
}

/// A lazy mise launcher: a command-wrapper or a shim. Running one for a tool that is not
/// installed yet STARTS A LONG INSTALL (it happened with `grok` and `agy`), so a probe
/// must never execute one unless mise already has that tool installed.
fn is_mise_wrapper(path: &Path) -> bool {
    let parts: Vec<&std::ffi::OsStr> = path.components().map(|c| c.as_os_str()).collect();
    parts.iter().any(|c| *c == "command-wrappers")
        || parts.windows(2).any(|w| w[0] == "mise" && w[1] == "shims")
}

/// The pure decision: `bin` is really installed when some candidate on PATH is a real
/// binary, or when every candidate is a lazy launcher but mise already has the tool.
fn really_installed(candidates: &[PathBuf], mise_has_tool: bool) -> bool {
    candidates.iter().any(|p| !is_mise_wrapper(p)) || (!candidates.is_empty() && mise_has_tool)
}

/// Whether running `bin` is safe for a probe: a real binary, never a lazy launcher that
/// would install it. Use this, not [`on_path`], before executing any provider CLI.
pub(crate) fn installed_for_real(bin: &str) -> bool {
    if bin.contains('/') {
        return is_executable_file(Path::new(bin)) && !is_mise_wrapper(Path::new(bin));
    }
    let candidates = find_on_path(bin, &path_dirs(), &is_executable_file);
    let data = mise_data_dir(std::env::var("MISE_DATA_DIR").ok(), std::env::var("XDG_DATA_HOME").ok(), home());
    really_installed(&candidates, data.is_some_and(|d| mise_installs_binary(&d, bin)))
}

/// mise's data directory: `$MISE_DATA_DIR`, else `$XDG_DATA_HOME/mise`, else
/// `~/.local/share/mise`.
fn mise_data_dir(mise_data_dir: Option<String>, xdg_data_home: Option<String>, home: Option<PathBuf>) -> Option<PathBuf> {
    mise_data_dir
        .map(PathBuf::from)
        .or_else(|| xdg_data_home.map(|x| PathBuf::from(x).join("mise")))
        .or_else(|| home.map(|h| h.join(".local/share/mise")))
}

/// True when some mise install holds an executable file called `bin`: `installs/<tool>/<version>/<bin>`
/// or `installs/<tool>/<version>/bin/<bin>`, whatever the tool directory is called (a tool
/// installed through a backend is `npm-google-gemini-cli`, its binary `gemini`).
///
/// Found from the filesystem alone, and a directory is not enough: mise creates an EMPTY
/// version directory when an install is started, and Omarchy's account wrapper runs `mise
/// install` (or `mise use -g`) whenever it finds no real binary, so an empty or half-done
/// install must count as "not installed". Nothing is executed to find out.
fn mise_installs_binary(data_dir: &Path, bin: &str) -> bool {
    let subdirs = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir).map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default()
    };
    subdirs(&data_dir.join("installs"))
        .iter()
        .flat_map(|tool| subdirs(tool))
        .any(|version| [version.join(bin), version.join("bin").join(bin)].iter().any(|exe| is_executable_file(exe)))
}

async fn probe_grok() -> Availability {
    decide_grok(GrokFacts {
        real_binary: installed_for_real("grok"),
        home_dir: home().is_some_and(|h| grok::session_dir_exists(&h)),
        // The same rule the provider factory uses to pass the key on, so "ready" means it reaches the CLI.
        api_key: !grok::auth_env(&envfile::lookup).is_empty(),
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
        assert_eq!(decide_grok(GrokFacts { real_binary: false, home_dir: true, api_key: true }).state, State::NotInstalled);
        assert_eq!(decide_grok(GrokFacts { real_binary: true, ..Default::default() }).state, State::NotLoggedIn);
        let a = decide_grok(GrokFacts { real_binary: true, home_dir: true, ..Default::default() });
        assert_eq!((a.state, a.verified), (State::Ready, false));
        // An API key alone is enough: no `grok login`, no ~/.grok.
        let k = decide_grok(GrokFacts { real_binary: true, api_key: true, ..Default::default() });
        assert_eq!((k.state, k.verified), (State::Ready, false));
        assert!(k.detail.contains("XAI_API_KEY"), "{}", k.detail);
    }

    #[test]
    fn probes_run_in_mise_offline_mode_so_they_never_update_or_install() {
        let cmd = probe_command("codex", &["login", "status"]);
        let env: Vec<_> = cmd.as_std().get_envs().collect();
        assert!(env.iter().any(|(k, v)| *k == "MISE_OFFLINE" && *v == Some(std::ffi::OsStr::new("1"))), "{env:?}");
    }

    #[test]
    fn lazy_launchers_are_never_counted_as_installed() {
        let wrapper = PathBuf::from("/h/.local/share/mise/command-wrappers/bin/codex");
        let shim = PathBuf::from("/h/.local/share/mise/shims/agy");
        let real = PathBuf::from("/usr/bin/codex");
        assert!(is_mise_wrapper(&shim), "mise shims install on first run too (the agy accident)");
        assert!(!is_mise_wrapper(Path::new("/opt/shims/tool")), "only mise's shims directory");
        // Only a lazy launcher and mise has nothing installed: never run it.
        assert!(!really_installed(std::slice::from_ref(&wrapper), false));
        assert!(!really_installed(std::slice::from_ref(&shim), false));
        // A lazy launcher for a tool mise already installed is safe to run.
        assert!(really_installed(std::slice::from_ref(&wrapper), true));
        // A real binary anywhere on PATH is installed.
        assert!(really_installed(&[shim, real], false));
        // Nothing on PATH is never installed, whatever mise says.
        assert!(!really_installed(&[], true));
    }

    /// A mise data directory in a fresh temp folder.
    fn mise_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-mise-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("installs")).unwrap();
        dir
    }

    #[cfg(unix)]
    fn write_file(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(not(unix))]
    fn write_file(path: &Path, _mode: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
    }

    #[test]
    fn an_install_is_found_by_its_binary_whatever_the_tool_directory_is_called() {
        let data = mise_dir("layouts");
        // The directory is named like the binary, the binary in the version directory.
        write_file(&data.join("installs/claude/2.1.3/claude"), 0o755);
        assert!(mise_installs_binary(&data, "claude"));
        // Named like the binary, in `bin/`.
        write_file(&data.join("installs/codex/0.1.0/bin/codex"), 0o755);
        assert!(mise_installs_binary(&data, "codex"));
        // A backend-prefixed tool name: `npm:@google/gemini-cli` is `npm-google-gemini-cli`.
        write_file(&data.join("installs/npm-google-gemini-cli/0.9.0/bin/gemini"), 0o755);
        assert!(mise_installs_binary(&data, "gemini"));
        // A binary that differs from the tool (`agy` from `antigravity-cli`).
        write_file(&data.join("installs/antigravity-cli/1.0.0/bin/agy"), 0o755);
        assert!(mise_installs_binary(&data, "agy"));
        // Other binaries are not found just because something is installed.
        assert!(!mise_installs_binary(&data, "grok"));
        assert!(!mise_installs_binary(&data.join("missing"), "claude"));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn an_empty_version_directory_or_a_non_executable_file_is_not_an_install() {
        let data = mise_dir("hollow");
        // `mise install` started and did not finish (it happened with grok): a version
        // directory exists, with nothing in it.
        std::fs::create_dir_all(data.join("installs/grok/1.0.77")).unwrap();
        assert!(!mise_installs_binary(&data, "grok"));
        // Files that are not the binary.
        write_file(&data.join("installs/grok/1.0.77/README"), 0o644);
        assert!(!mise_installs_binary(&data, "grok"));
        // The right name but not executable.
        write_file(&data.join("installs/gemini/0.1.0/bin/gemini"), 0o644);
        assert!(!mise_installs_binary(&data, "gemini"));
        // A directory with the binary's name is not a binary either.
        std::fs::create_dir_all(data.join("installs/agy/1.0.0/bin/agy")).unwrap();
        assert!(!mise_installs_binary(&data, "agy"));
        // And once it really is installed, it counts.
        write_file(&data.join("installs/grok/1.0.77/bin/grok"), 0o755);
        assert!(mise_installs_binary(&data, "grok"));
        let _ = std::fs::remove_dir_all(&data);
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

    // ---- the slow-probe retry

    fn unknown(p: ProviderId) -> Availability {
        avail(p, State::Unknown, "probe timed out after 5s")
    }

    #[test]
    fn only_providers_that_could_not_tell_are_retried() {
        let first = [ready(ProviderId::Claude, None), unknown(ProviderId::Codex), avail(ProviderId::Gemini, State::NotLoggedIn, "x"), unknown(ProviderId::Grok)];
        assert_eq!(retry_candidates(&first), [ProviderId::Codex, ProviderId::Grok]);
        assert!(retry_candidates(&[ready(ProviderId::Claude, None)]).is_empty());
    }

    #[tokio::test]
    async fn the_retry_asks_each_provider_once_under_the_cap_and_reports_answers_as_they_come() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        // codex answers after 20 ms; claude never answers; both under a 150 ms cap.
        let fake = move |p: ProviderId| {
            counter.fetch_add(1, Ordering::SeqCst);
            async move {
                if p == ProviderId::Codex {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    ready(p, None)
                } else {
                    std::future::pending().await
                }
            }
        };
        let mut answers = Vec::new();
        retry_unknown(vec![ProviderId::Claude, ProviderId::Codex], Duration::from_millis(150), fake, |a| answers.push((a.provider, a.state))).await;
        assert_eq!(answers, [(ProviderId::Codex, State::Ready), (ProviderId::Claude, State::Unknown)], "the quick one is not held back by the slow one");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "one retry each, never a loop");
    }

    #[tokio::test]
    async fn a_retry_with_nothing_to_retry_does_nothing() {
        let mut seen = 0;
        retry_unknown(Vec::new(), RETRY_CAP, |p| async move { ready(p, None) }, |_| seen += 1).await;
        assert_eq!(seen, 0);
    }

    #[test]
    fn a_fresh_answer_replaces_a_cached_one_only_when_it_differs_and_is_definite() {
        let mut cached = vec![ready(ProviderId::Claude, None), avail(ProviderId::Codex, State::NotLoggedIn, "x")];
        assert!(!merge_refresh(&mut cached, ready(ProviderId::Claude, None)), "same answer");
        assert!(!merge_refresh(&mut cached, avail(ProviderId::Claude, State::Unknown, "slow")), "unknown teaches nothing");
        assert!(merge_refresh(&mut cached, avail(ProviderId::Claude, State::NotLoggedIn, "logged out")));
        assert!(merge_refresh(&mut cached, ready(ProviderId::Codex, None)));
        assert_eq!((cached[0].state, cached[1].state), (State::NotLoggedIn, State::Ready));
    }

    #[test]
    fn a_late_answer_fills_in_only_a_provider_that_is_still_unknown() {
        let mut now = vec![ready(ProviderId::Claude, None), unknown(ProviderId::Codex), avail(ProviderId::Gemini, State::NotLoggedIn, "x")];
        assert!(merge_late(&mut now, ready(ProviderId::Codex, Some(vec!["m"]))));
        assert_eq!((now[1].state, now[1].models.clone()), (State::Ready, Some(vec!["m".to_string()])));
        assert!(!merge_late(&mut now, avail(ProviderId::Codex, State::NotLoggedIn, "x")), "a definite answer is never overwritten");
        assert!(!merge_late(&mut now, ready(ProviderId::Gemini, None)), "nor is a refresh's result");
        assert_eq!(now[2].state, State::NotLoggedIn);
        let mut still = vec![unknown(ProviderId::Codex)];
        assert!(!merge_late(&mut still, unknown(ProviderId::Codex)), "Unknown teaches nothing");
    }
}
