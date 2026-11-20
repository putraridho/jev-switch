//! `doctor`: what is set up, what is not, and the exact thing to do about each gap.
//!
//! The decision logic is pure: [`diagnose`] turns an [`Inputs`] snapshot into a
//! [`Report`] of structured [`Check`]s, and [`first_run_hint`] turns provider
//! availability into the short startup nudge. Neither does IO, so both are
//! table-tested. Only [`gather`] touches the machine (file metadata, a scratch write,
//! `git`, one tiny Jev request), and it never panics: a failed probe is a fact
//! (`Some(false)`), not an error. Colors and layout belong to whoever renders the report.

use std::cmp::Reverse;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use crate::availability::{Availability, State};
use crate::backend::ProviderId;
use crate::cooldown::Cooldowns;
use crate::providers::{format_left, BlockReason, Runtime, ALL_PROVIDERS};
use crate::typesafe::TypeSafe;

/// Where keys live when `$JEV_ROUTER_ENV` does not say otherwise; shown in fixes.
const DEFAULT_ENV_PATH: &str = "~/.jev-router.env";

/// The Jev ping and the `git` check each get this long, so `doctor` never hangs.
const IO_CAP: Duration = Duration::from_secs(3);

const UNVERIFIED_NOTE: &str = "unverified: this check was written from documentation, not tried against the real CLI";

// ------------------------------------------------------------------ report types

/// Most severe is greatest, so sorting descending puts Problems first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Ok,
    Info,
    Warn,
    Problem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub id: &'static str,
    pub title: String,
    pub severity: Severity,
    pub detail: String,
    /// Exact commands or steps, in order. Empty when there is nothing to do.
    pub fix: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Problems first, then Warns, then Infos, then Oks; stable within a severity.
    pub checks: Vec<Check>,
    pub ready_providers: Vec<ProviderId>,
    /// One plain sentence on what routing can do right now.
    pub routing_note: String,
}

/// What `~/.jev-router.env` (or `$JEV_ROUTER_ENV`) looks like on disk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvFileFacts {
    pub path: PathBuf,
    /// Permission bits (`mode & 0o777`); `None` where the platform has none.
    pub mode_bits: Option<u32>,
    pub has_keys: bool,
}

/// Every fact [`diagnose`] needs. `None` means "not checked": the check is left out
/// rather than guessed.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub avail: Vec<Availability>,
    pub jev_key_present: bool,
    pub jev_reachable: Option<bool>,
    pub in_git_repo: Option<bool>,
    pub state_dir_writable: Option<bool>,
    pub env_file: Option<EnvFileFacts>,
    /// Active cooldowns, one human line each ("claude cooling for 12m").
    pub cooldown_lines: Vec<String>,
    /// Providers skipped for this session, with why.
    pub blocked: Vec<(ProviderId, String)>,
    pub allow_credits: bool,
}

// ---------------------------------------------------------------------- diagnose

/// The whole report. Pure.
pub fn diagnose(inputs: &Inputs) -> Report {
    let env_path = env_path_text(inputs);
    let ready = ready_providers(inputs);

    let mut checks: Vec<Check> = Vec::new();
    checks.extend(no_provider_check(&ready));
    checks.push(jev_key_check(inputs.jev_key_present, &env_path));
    checks.extend(jev_reachable_check(inputs));
    checks.extend(inputs.avail.iter().map(|a| provider_check(a, &env_path)));
    checks.extend(git_check(inputs.in_git_repo));
    checks.extend(state_dir_check(inputs.state_dir_writable));
    checks.extend(env_perms_check(inputs.env_file.as_ref()));
    checks.extend(credits_check(inputs));
    checks.extend(cooldown_check(&inputs.cooldown_lines));
    checks.extend(blocked_check(&inputs.blocked));

    // Stable, so the construction order above breaks ties within a severity.
    checks.sort_by_key(|c| Reverse(c.severity));

    let routing_note = routing_note(&ready);
    Report { checks, ready_providers: ready, routing_note }
}

/// Ready providers that are not blocked for this session, in availability order.
fn ready_providers(inputs: &Inputs) -> Vec<ProviderId> {
    inputs
        .avail
        .iter()
        .filter(|a| a.state == State::Ready)
        .map(|a| a.provider)
        .filter(|p| !inputs.blocked.iter().any(|(b, _)| b == p))
        .collect()
}

fn routing_note(ready: &[ProviderId]) -> String {
    match ready {
        [] => "No provider is ready, so nothing can run yet. Set one up using the steps below.".to_string(),
        [only] => format!("Only {only} is ready, so every turn runs on {only}. Add a second provider to route by task type."),
        many => format!("Routing across {}.", many.iter().map(ProviderId::as_str).collect::<Vec<_>>().join(", ")),
    }
}

fn env_path_text(inputs: &Inputs) -> String {
    inputs.env_file.as_ref().map_or_else(|| DEFAULT_ENV_PATH.to_string(), |f| f.path.display().to_string())
}

fn check(id: &'static str, title: impl Into<String>, severity: Severity, detail: impl Into<String>, fix: Vec<String>) -> Check {
    Check { id, title: title.into(), severity, detail: detail.into(), fix }
}

fn steps(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

// ------------------------------------------------------------------ provider checks

fn no_provider_check(ready: &[ProviderId]) -> Option<Check> {
    ready.is_empty().then(|| {
        check(
            "no-provider",
            "No provider is ready",
            Severity::Problem,
            "Every turn needs at least one provider that is installed and logged in.",
            steps(&["Set up one provider using the steps for it below"]),
        )
    })
}

fn provider_check(a: &Availability, env_path: &str) -> Check {
    let name = a.provider.as_str();
    let (severity, title, mut fix) = match a.state {
        State::Ready => (Severity::Ok, format!("{name} is ready"), Vec::new()),
        State::NotInstalled => (Severity::Info, format!("{name} is not installed"), fix_steps(a.provider, a.state, env_path)),
        State::NotLoggedIn => (Severity::Warn, format!("{name} is installed but not logged in"), fix_steps(a.provider, a.state, env_path)),
        State::NoKey => (Severity::Warn, format!("{name} has no API key"), fix_steps(a.provider, a.state, env_path)),
        State::Unknown => (Severity::Warn, format!("{name}: could not tell if it is usable"), fix_steps(a.provider, a.state, env_path)),
    };
    fix.dedup();
    check(provider_id(a.provider), title, severity, provider_detail(a), fix)
}

/// One stable id per provider, for renderers that want to find a check.
fn provider_id(p: ProviderId) -> &'static str {
    match p {
        ProviderId::Claude => "provider-claude",
        ProviderId::DeepSeek => "provider-deepseek",
        ProviderId::Codex => "provider-codex",
        ProviderId::Gemini => "provider-gemini",
        ProviderId::Grok => "provider-grok",
    }
}

/// State, plan, models and the probe's own words, plus an unverified label.
fn provider_detail(a: &Availability) -> String {
    let mut parts: Vec<String> = Vec::new();
    if a.state == State::Ready {
        if let Some(plan) = &a.plan {
            parts.push(format!("plan: {plan}"));
        }
        match &a.models {
            Some(models) if !models.is_empty() => parts.push(format!("models: {}", models.join(", "))),
            _ => parts.push("models: all the router knows for it".to_string()),
        }
    }
    if !a.detail.is_empty() {
        parts.push(a.detail.clone());
    }
    if a.provider == ProviderId::DeepSeek && a.state != State::NotInstalled {
        parts.push("billed per token; adding a key creates no subscription".to_string());
    }
    if !a.verified {
        parts.push(format!("({UNVERIFIED_NOTE})"));
    }
    parts.join("; ")
}

/// The exact steps to get `provider` from `state` to ready. Installing comes first,
/// then logging in. Steps that are commands are the command alone; anything not
/// tried against the real CLI says "unverified".
fn fix_steps(provider: ProviderId, state: State, env_path: &str) -> Vec<String> {
    match state {
        State::Ready => Vec::new(),
        State::NotInstalled => [install_steps(provider), login_steps(provider, env_path)].concat(),
        State::NotLoggedIn | State::NoKey => login_steps(provider, env_path),
        State::Unknown => unknown_steps(provider),
    }
}

/// How to install the CLI behind `provider`: the one source of install hints.
pub fn install_hint(provider: ProviderId) -> &'static str {
    match provider {
        // DeepSeek is the claude CLI pointed at another API.
        ProviderId::Claude | ProviderId::DeepSeek => "Install Claude Code: https://code.claude.com/docs/en/quickstart",
        ProviderId::Codex => "npm i -g @openai/codex (or see the vendor's install page)",
        ProviderId::Gemini => "npm i -g @google/gemini-cli",
        ProviderId::Grok => {
            "Install the Grok CLI: see the vendor's install page (if `grok` is a lazy mise wrapper, running `grok` once installs it)"
        }
    }
}

/// The command that logs in to `provider`: the one source of login commands, used by
/// `doctor`, the first-run hint and every error message. `None` for DeepSeek, which has
/// no login: it takes an API key in the env file. Only `claude auth login` and `codex
/// login` were tried against the real CLIs ([`login_verified`]).
pub fn login_command(provider: ProviderId) -> Option<&'static str> {
    match provider {
        ProviderId::Claude => Some("claude auth login"),
        ProviderId::Codex => Some("codex login"),
        // Running the CLI starts its own sign-in flow.
        ProviderId::Gemini => Some("gemini"),
        ProviderId::Grok => Some("grok"),
        ProviderId::DeepSeek => None,
    }
}

/// Whether [`login_command`] for `provider` was run against the real CLI.
pub fn login_verified(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::Claude | ProviderId::Codex)
}

/// [`login_command`] as one sentence for a message: `run `codex login``.
pub fn login_instruction(provider: ProviderId) -> String {
    match (provider, login_command(provider)) {
        (ProviderId::Gemini, Some(cmd)) => format!("run `{cmd}` and choose a login"),
        (ProviderId::Grok, Some(cmd)) => format!("run `{cmd}` and sign in"),
        (_, Some(cmd)) => format!("run `{cmd}`"),
        (_, None) => format!("add `DEEPSEEK_API_KEY` to {DEFAULT_ENV_PATH}"),
    }
}

fn install_steps(provider: ProviderId) -> Vec<String> {
    vec![install_hint(provider).to_string()]
}

fn login_steps(provider: ProviderId, env_path: &str) -> Vec<String> {
    let cmd = login_command(provider).unwrap_or_default();
    match provider {
        ProviderId::Claude => vec![cmd.to_string()],
        ProviderId::Codex => vec![cmd.to_string(), format!("{cmd} --device-auth (if this machine is headless)")],
        ProviderId::Gemini => vec![
            format!("Run `{cmd}` once and choose a login"),
            format!("or set GEMINI_API_KEY=<key> in {env_path} (unverified: the probe reads an exported GEMINI_API_KEY today; if Gemini stays not ready, `export GEMINI_API_KEY=<key>` in your shell)"),
            "unverified: Antigravity users run `agy` and sign in".to_string(),
        ],
        ProviderId::Grok => vec![
            format!("{cmd} (first run installs it, which can take minutes)"),
            "Sign in when it asks (unverified; needs SuperGrok or X Premium+, the $8 plan likely lacks the CLI)".to_string(),
            format!("or set XAI_API_KEY=<key> in {env_path} (billed per token by xAI)"),
        ],
        ProviderId::DeepSeek => vec![format!("Add DEEPSEEK_API_KEY=<key> to {env_path} (billed per token)")],
    }
}

fn unknown_steps(provider: ProviderId) -> Vec<String> {
    let status = match provider {
        ProviderId::Claude => Some("claude auth status"),
        ProviderId::Codex => Some("codex login status"),
        _ => None,
    };
    status.into_iter().map(String::from).chain(["/providers refresh".to_string()]).collect()
}

// ----------------------------------------------------------------------- Jev checks

fn jev_key_check(present: bool, env_path: &str) -> Check {
    if present {
        check("jev-key", "Jev key found", Severity::Ok, "Routing can ask Jev which model fits each task.", Vec::new())
    } else {
        check(
            "jev-key",
            "Jev key is missing",
            Severity::Problem,
            "Routing needs a Jev key to judge each task.",
            vec![format!("Add JEV_API_KEY=... to {env_path}")],
        )
    }
}

/// Left out when the key is missing (nothing to test) or the ping was not run.
fn jev_reachable_check(inputs: &Inputs) -> Option<Check> {
    if !inputs.jev_key_present {
        return None;
    }
    Some(match inputs.jev_reachable? {
        true => check("jev-reachable", "Jev is reachable", Severity::Ok, "A test request was answered.", Vec::new()),
        false => check(
            "jev-reachable",
            "Jev did not answer",
            Severity::Warn,
            "A tiny test request failed or took longer than 3 seconds.",
            steps(&["Check your network connection", "Check that the key in JEV_API_KEY is valid and not expired"]),
        ),
    })
}

// ------------------------------------------------------------------ machine checks

fn git_check(in_repo: Option<bool>) -> Option<Check> {
    Some(match in_repo? {
        true => check("git-repo", "This folder is a git repository", Severity::Ok, "Codex and Gemini will run here; /undo and the change summary work.", Vec::new()),
        false => check(
            "git-repo",
            "This folder is not a git repository",
            Severity::Warn,
            "Codex and Gemini refuse non-git or untrusted folders, and /undo and the change summary need git.",
            steps(&["git init", "or start the router from inside a git repository"]),
        ),
    })
}

fn state_dir_check(writable: Option<bool>) -> Option<Check> {
    Some(match writable? {
        true => check("state-dir", "State directory is writable", Severity::Ok, "Logs, cooldowns and saved sessions can be kept.", Vec::new()),
        false => check(
            "state-dir",
            "State directory is not writable",
            Severity::Warn,
            "Logs, cooldowns and saved sessions need it; without it they are lost between runs.",
            steps(&["mkdir -p ~/.local/state/jev-router", "chmod u+rwx ~/.local/state/jev-router"]),
        ),
    })
}

/// A key file anyone else can read is a leak. Left out when there is no file or
/// nothing secret in it.
fn env_perms_check(file: Option<&EnvFileFacts>) -> Option<Check> {
    let f = file.filter(|f| f.has_keys)?;
    let path = f.path.display().to_string();
    Some(match f.mode_bits {
        Some(bits) if bits & 0o077 != 0 => check(
            "env-perms",
            format!("{path} is readable by other users"),
            Severity::Warn,
            format!("It holds keys and its mode is {:03o}; only you should be able to read it.", bits & 0o777),
            vec![format!("chmod 600 {path}")],
        ),
        _ => check("env-perms", format!("{path} is private"), Severity::Ok, "Only you can read the key file.", Vec::new()),
    })
}

// ------------------------------------------------------------------- state checks

/// Claude is ready, extra-credit models are off, and the plan is not Max (Max
/// includes them), so say why Fable-class models are skipped and how to opt in.
fn credits_check(inputs: &Inputs) -> Option<Check> {
    if inputs.allow_credits {
        return None;
    }
    let claude = inputs.avail.iter().find(|a| a.provider == ProviderId::Claude && a.state == State::Ready)?;
    if claude.plan.as_deref().is_some_and(|p| p.to_ascii_lowercase().contains("max")) {
        return None;
    }
    Some(check(
        "credits",
        "Extra-credit models are skipped",
        Severity::Info,
        "Models billed as extra credits on your Claude plan (such as Fable) are never picked unless you opt in.",
        steps(&["export JEV_ALLOW_CREDITS=1"]),
    ))
}

fn cooldown_check(lines: &[String]) -> Option<Check> {
    (!lines.is_empty()).then(|| {
        check("cooldowns", "Providers are cooling down", Severity::Info, lines.join("\n"), steps(&["Nothing to do: they come back on their own when the limit resets"]))
    })
}

fn blocked_check(blocked: &[(ProviderId, String)]) -> Option<Check> {
    (!blocked.is_empty()).then(|| {
        let detail = blocked.iter().map(|(p, why)| format!("{p}: {why}")).collect::<Vec<_>>().join("\n");
        check("blocked", "Providers are skipped for this session", Severity::Info, detail, Vec::new())
    })
}

// -------------------------------------------------------------------- first run hint

/// A short nudge for startup, or `None` when everything useful is ready. Shown when
/// fewer than two providers are ready (routing has nothing to choose between) or one
/// is installed but not logged in. Providers that are not installed are never
/// mentioned individually.
pub fn first_run_hint(avail: &[Availability]) -> Option<String> {
    let ready = avail.iter().filter(|a| a.state == State::Ready).count();
    let half_configured: Vec<&Availability> = avail
        .iter()
        .filter(|a| a.state == State::NotLoggedIn || (ready < 2 && a.state == State::NoKey))
        .collect();
    if ready >= 2 && half_configured.is_empty() {
        return None;
    }

    let mut lines: Vec<String> = Vec::new();
    match ready {
        0 => lines.push("No provider is ready yet, so nothing can run.".to_string()),
        1 => lines.push("Only one provider is ready, so there is nothing to route between.".to_string()),
        _ => {}
    }
    for a in half_configured {
        let first = fix_steps(a.provider, a.state, DEFAULT_ENV_PATH).into_iter().next().unwrap_or_default();
        let why = if a.state == State::NoKey { "has no API key" } else { "is installed but not logged in" };
        lines.push(format!("{} {why}: {first}", a.provider));
    }
    if ready == 0 && lines.len() == 1 {
        lines.push("Run doctor for the install steps.".to_string());
    }
    Some(lines.join("\n"))
}

// --------------------------------------------------------------------------- IO

/// Collect the facts. Thin IO: every probe is bounded and a failure becomes a fact.
pub async fn gather(runtime: &Runtime, dir: &Path) -> Inputs {
    let (jev_key_present, jev_reachable) = jev_status().await;
    Inputs {
        avail: runtime.avail.clone(),
        jev_key_present,
        jev_reachable,
        in_git_repo: in_git_repo(dir).await,
        state_dir_writable: Some(state_dir_writable()),
        env_file: env_file_facts(),
        cooldown_lines: cooldown_lines(runtime),
        blocked: blocked(runtime),
        allow_credits: runtime.allow.credits,
    }
}

/// `(key present, reachable)`. Reachable is not asked without a key.
async fn jev_status() -> (bool, Option<bool>) {
    let Ok(client) = TypeSafe::from_env() else { return (false, None) };
    let ping = client.choice(json!("ping"), "Pick option a.", json!({ "a": "the only real option", "b": "never pick this" }));
    let ok = matches!(tokio::time::timeout(IO_CAP, ping).await, Ok(Ok(_)));
    (true, Some(ok))
}

/// `Some(true/false)` from `git rev-parse`; `None` when git cannot be run at all.
async fn in_git_repo(dir: &Path) -> Option<bool> {
    let run = tokio::process::Command::new("git")
        .args(crate::gitflow::no_repo_programs(dir))
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .status();
    match tokio::time::timeout(IO_CAP, run).await {
        Ok(Ok(status)) => Some(status.success()),
        _ => None,
    }
}

/// Create the state directory if needed and write then remove a scratch file.
fn state_dir_writable() -> bool {
    let Some(dir) = crate::log::state_dir() else { return false };
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".doctor-probe-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn env_file_facts() -> Option<EnvFileFacts> {
    let path = crate::envfile::env_file_path(std::env::var("JEV_ROUTER_ENV").ok(), std::env::var("HOME").ok())?;
    let meta = std::fs::metadata(&path).ok().filter(|m| m.is_file())?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    Some(EnvFileFacts { path, mode_bits: mode_bits(&meta), has_keys: file_has_keys(&text) })
}

#[cfg(unix)]
fn mode_bits(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode_bits(_meta: &std::fs::Metadata) -> Option<u32> {
    None
}

/// True when some `KEY=value` line has a non-empty value (comments and blanks do not count).
fn file_has_keys(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim().trim_start_matches("export ").trim();
        !line.starts_with('#')
            && line.split_once('=').is_some_and(|(k, v)| !k.trim().is_empty() && !v.trim().trim_matches(|c| c == '"' || c == '\'').is_empty())
    })
}

/// In-memory cooldowns plus whatever other processes saved.
fn cooldown_lines(runtime: &Runtime) -> Vec<String> {
    let now = now_ms();
    let saved = runtime.cooldown_path.as_deref().map(Cooldowns::load).unwrap_or_default();
    let all = runtime.cooldowns.union(&saved);
    ALL_PROVIDERS.iter().filter_map(|p| Some(format!("{p} cooling for {}", format_left(all.active(*p, now)?)))).collect()
}

fn blocked(runtime: &Runtime) -> Vec<(ProviderId, String)> {
    ALL_PROVIDERS
        .iter()
        .filter_map(|p| Some((*p, block_reason_text(*p, *runtime.blocked.get(p)?))))
        .collect()
}

fn block_reason_text(provider: ProviderId, reason: BlockReason) -> String {
    match reason {
        BlockReason::Auth => "its login failed; log in again, then /providers refresh".to_string(),
        BlockReason::UnusableHere => match provider {
            ProviderId::Codex => "needs a git repository here; /new or another folder clears it".to_string(),
            ProviderId::Gemini => "needs a trusted workspace here; /new or another folder clears it".to_string(),
            _ => "refuses to run in this directory; /new or another folder clears it".to_string(),
        },
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

// -------------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

    fn av(provider: ProviderId, state: State) -> Availability {
        let verified = matches!(provider, Claude | Codex | DeepSeek);
        Availability { provider, state, plan: None, models: None, detail: String::new(), verified }
    }

    fn all(states: [State; 5]) -> Vec<Availability> {
        [Claude, DeepSeek, Codex, Gemini, Grok].into_iter().zip(states).map(|(p, s)| av(p, s)).collect()
    }

    fn inputs(avail: Vec<Availability>) -> Inputs {
        Inputs { avail, jev_key_present: true, jev_reachable: Some(true), ..Inputs::default() }
    }

    fn find<'a>(r: &'a Report, id: &str) -> &'a Check {
        r.checks.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no check {id}"))
    }

    fn has_check(r: &Report, id: &str) -> bool {
        r.checks.iter().any(|c| c.id == id)
    }

    // ---- per provider, per state

    #[test]
    fn provider_state_table_gives_severity_and_exact_first_fix() {
        use State::*;
        // (provider, state, severity, a step that must be present verbatim)
        let table: &[(ProviderId, State, Severity, &str)] = &[
            (Claude, Ready, Severity::Ok, ""),
            (Claude, NotLoggedIn, Severity::Warn, "claude auth login"),
            (Claude, NotInstalled, Severity::Info, "Install Claude Code: https://code.claude.com/docs/en/quickstart"),
            (Claude, Unknown, Severity::Warn, "claude auth status"),
            (Codex, Ready, Severity::Ok, ""),
            (Codex, NotLoggedIn, Severity::Warn, "codex login"),
            (Codex, NotLoggedIn, Severity::Warn, "codex login --device-auth (if this machine is headless)"),
            (Codex, NotInstalled, Severity::Info, "npm i -g @openai/codex (or see the vendor's install page)"),
            (Codex, Unknown, Severity::Warn, "codex login status"),
            (Gemini, Ready, Severity::Ok, ""),
            (Gemini, NotLoggedIn, Severity::Warn, "Run `gemini` once and choose a login"),
            (Gemini, NotInstalled, Severity::Info, "npm i -g @google/gemini-cli"),
            (Grok, Ready, Severity::Ok, ""),
            (Grok, NotLoggedIn, Severity::Warn, "grok (first run installs it, which can take minutes)"),
            (Grok, NotInstalled, Severity::Info, "Install the Grok CLI"),
            (DeepSeek, Ready, Severity::Ok, ""),
            (DeepSeek, NoKey, Severity::Warn, "Add DEEPSEEK_API_KEY=<key> to ~/.jev-router.env (billed per token)"),
            (DeepSeek, NotInstalled, Severity::Info, "Install Claude Code: https://code.claude.com/docs/en/quickstart"),
        ];
        for (provider, state, severity, step) in table {
            let r = diagnose(&inputs(vec![av(*provider, *state)]));
            let c = find(&r, provider_id(*provider));
            assert_eq!(c.severity, *severity, "{provider} {state:?}");
            if step.is_empty() {
                assert!(c.fix.is_empty(), "{provider} ready needs no fix");
            } else {
                assert!(c.fix.iter().any(|s| s.starts_with(step)), "{provider} {state:?} fix {:?} lacks {step}", c.fix);
            }
        }
    }

    #[test]
    fn every_provider_state_combination_yields_one_check_without_panicking() {
        use State::*;
        for provider in ALL_PROVIDERS {
            for state in [Ready, NotInstalled, NotLoggedIn, NoKey, Unknown] {
                let r = diagnose(&inputs(vec![av(provider, state)]));
                assert_eq!(r.checks.iter().filter(|c| c.id == provider_id(provider)).count(), 1);
            }
        }
    }

    #[test]
    fn not_installed_gives_install_then_login_steps() {
        let r = diagnose(&inputs(vec![av(Codex, State::NotInstalled)]));
        let fix = &find(&r, "provider-codex").fix;
        assert!(fix[0].starts_with("npm i -g @openai/codex"));
        assert_eq!(fix[1], "codex login");
    }

    #[test]
    fn gemini_fix_offers_key_or_login_and_flags_agy_unverified() {
        let r = diagnose(&inputs(vec![av(Gemini, State::NotLoggedIn)]));
        let fix = find(&r, "provider-gemini").fix.join("\n");
        assert!(fix.contains("GEMINI_API_KEY=<key> in ~/.jev-router.env"));
        assert!(fix.contains("unverified: Antigravity users run `agy` and sign in"));
    }

    #[test]
    fn grok_fix_mentions_the_slow_first_run_and_the_plan_caveat() {
        let r = diagnose(&inputs(vec![av(Grok, State::NotLoggedIn)]));
        let fix = find(&r, "provider-grok").fix.join("\n");
        assert!(fix.contains("which can take minutes"));
        assert!(fix.contains("SuperGrok or X Premium+"));
        assert!(fix.contains("unverified"));
    }

    #[test]
    fn deepseek_says_per_token_and_no_subscription() {
        let r = diagnose(&inputs(vec![av(DeepSeek, State::NoKey)]));
        let c = find(&r, "provider-deepseek");
        assert!(c.detail.contains("billed per token"));
        assert!(c.detail.contains("no subscription"));
    }

    #[test]
    fn unverified_providers_are_labelled_and_verified_ones_are_not() {
        for p in ALL_PROVIDERS {
            let r = diagnose(&inputs(vec![av(p, State::Ready)]));
            let labelled = find(&r, provider_id(p)).detail.contains("unverified");
            assert_eq!(labelled, matches!(p, Gemini | Grok), "{p}");
        }
    }

    #[test]
    fn ready_detail_carries_plan_and_models() {
        let a = Availability { plan: Some("plus".into()), models: Some(vec!["m1".into(), "m2".into()]), ..av(Codex, State::Ready) };
        let r = diagnose(&inputs(vec![a]));
        let d = &find(&r, "provider-codex").detail;
        assert!(d.contains("plan: plus") && d.contains("models: m1, m2"), "{d}");
    }

    #[test]
    fn env_path_override_shows_up_in_key_fixes() {
        let mut i = inputs(vec![av(DeepSeek, State::NoKey)]);
        i.env_file = Some(EnvFileFacts { path: "/x/keys.env".into(), mode_bits: Some(0o600), has_keys: true });
        let r = diagnose(&i);
        assert!(find(&r, "provider-deepseek").fix[0].contains("/x/keys.env"));
    }

    // ---- routing note and readiness

    #[test]
    fn routing_note_by_ready_count() {
        use State::*;
        let none = diagnose(&inputs(all([NotInstalled; 5])));
        assert!(none.routing_note.starts_with("No provider is ready"));
        let one = diagnose(&inputs(all([Ready, NoKey, NotInstalled, NotInstalled, NotInstalled])));
        assert_eq!(one.routing_note, "Only claude is ready, so every turn runs on claude. Add a second provider to route by task type.");
        let two = diagnose(&inputs(all([Ready, NoKey, Ready, NotInstalled, NotInstalled])));
        assert_eq!(two.routing_note, "Routing across claude, codex.");
        assert_eq!(two.ready_providers, vec![Claude, Codex]);
    }

    #[test]
    fn a_blocked_provider_is_not_counted_ready() {
        let mut i = inputs(all([State::Ready, State::NoKey, State::Ready, State::NotInstalled, State::NotInstalled]));
        i.blocked = vec![(Codex, "its login failed".into())];
        let r = diagnose(&i);
        assert_eq!(r.ready_providers, vec![Claude]);
        assert!(find(&r, "blocked").detail.contains("codex: its login failed"));
    }

    #[test]
    fn no_ready_provider_is_a_problem() {
        let r = diagnose(&inputs(all([State::NotInstalled; 5])));
        assert_eq!(find(&r, "no-provider").severity, Severity::Problem);
        let r = diagnose(&inputs(all([State::Ready, State::NoKey, State::NotInstalled, State::NotInstalled, State::NotInstalled])));
        assert!(!has_check(&r, "no-provider"));
    }

    // ---- Jev

    #[test]
    fn missing_jev_key_is_a_problem_with_the_exact_fix_and_skips_reachability() {
        let mut i = inputs(vec![av(Claude, State::Ready)]);
        i.jev_key_present = false;
        i.jev_reachable = None;
        let r = diagnose(&i);
        let c = find(&r, "jev-key");
        assert_eq!(c.severity, Severity::Problem);
        assert_eq!(c.fix, vec!["Add JEV_API_KEY=... to ~/.jev-router.env".to_string()]);
        assert!(!has_check(&r, "jev-reachable"));
    }

    #[test]
    fn jev_reachability_table() {
        for (reachable, expect) in [(Some(true), Some(Severity::Ok)), (Some(false), Some(Severity::Warn)), (None, None)] {
            let mut i = inputs(vec![av(Claude, State::Ready)]);
            i.jev_reachable = reachable;
            let r = diagnose(&i);
            assert_eq!(r.checks.iter().find(|c| c.id == "jev-reachable").map(|c| c.severity), expect);
        }
    }

    // ---- machine checks

    #[test]
    fn git_and_state_dir_tables() {
        for (value, expect) in [(Some(true), Some(Severity::Ok)), (Some(false), Some(Severity::Warn)), (None, None)] {
            let mut i = inputs(vec![av(Claude, State::Ready)]);
            i.in_git_repo = value;
            i.state_dir_writable = value;
            let r = diagnose(&i);
            assert_eq!(r.checks.iter().find(|c| c.id == "git-repo").map(|c| c.severity), expect);
            assert_eq!(r.checks.iter().find(|c| c.id == "state-dir").map(|c| c.severity), expect);
        }
    }

    #[test]
    fn non_git_folder_warns_with_the_reason_and_fix() {
        let mut i = inputs(vec![]);
        i.in_git_repo = Some(false);
        let c = diagnose(&i).checks.into_iter().find(|c| c.id == "git-repo").unwrap();
        assert!(c.detail.contains("Codex and Gemini") && c.detail.contains("/undo"));
        assert_eq!(c.fix[0], "git init");
    }

    #[test]
    fn env_file_permission_table() {
        // (mode, has_keys, expected severity or None when the check is left out)
        let table = [
            (Some(0o600), true, Some(Severity::Ok)),
            (Some(0o400), true, Some(Severity::Ok)),
            (Some(0o640), true, Some(Severity::Warn)),
            (Some(0o604), true, Some(Severity::Warn)),
            (Some(0o644), true, Some(Severity::Warn)),
            (Some(0o644), false, None),
            (None, true, Some(Severity::Ok)),
        ];
        for (mode, has_keys, expect) in table {
            let mut i = inputs(vec![]);
            i.env_file = Some(EnvFileFacts { path: "/h/.jev-router.env".into(), mode_bits: mode, has_keys });
            let r = diagnose(&i);
            assert_eq!(r.checks.iter().find(|c| c.id == "env-perms").map(|c| c.severity), expect, "{mode:?} {has_keys}");
        }
    }

    #[test]
    fn loose_env_file_gets_the_chmod_fix() {
        let mut i = inputs(vec![]);
        i.env_file = Some(EnvFileFacts { path: "/h/.jev-router.env".into(), mode_bits: Some(0o644), has_keys: true });
        let r = diagnose(&i);
        assert_eq!(find(&r, "env-perms").fix, vec!["chmod 600 /h/.jev-router.env".to_string()]);
        assert!(find(&r, "env-perms").detail.contains("644"));
    }

    #[test]
    fn file_has_keys_ignores_comments_blanks_and_empty_values() {
        assert!(file_has_keys("JEV_API_KEY=abc\n"));
        assert!(file_has_keys("# note\nexport X='y'\n"));
        assert!(!file_has_keys(""));
        assert!(!file_has_keys("# A=1\n\nB=\nC=\"\"\nno equals\n"));
    }

    // ---- state checks

    #[test]
    fn credits_note_appears_only_for_ready_non_max_claude_without_opt_in() {
        let with = |plan: Option<&str>, state, allow| {
            let mut i = inputs(vec![Availability { plan: plan.map(String::from), ..av(Claude, state) }]);
            i.allow_credits = allow;
            has_check(&diagnose(&i), "credits")
        };
        assert!(with(Some("pro"), State::Ready, false));
        assert!(with(None, State::Ready, false));
        assert!(!with(Some("max_5x"), State::Ready, false));
        assert!(!with(Some("pro"), State::Ready, true));
        assert!(!with(Some("pro"), State::NotLoggedIn, false));
        let mut i = inputs(vec![Availability { plan: Some("pro".into()), ..av(Claude, State::Ready) }]);
        i.allow_credits = false;
        let r = diagnose(&i);
        let c = find(&r, "credits");
        assert_eq!(c.severity, Severity::Info);
        assert_eq!(c.fix, vec!["export JEV_ALLOW_CREDITS=1".to_string()]);
    }

    #[test]
    fn cooldowns_become_one_info_check_with_the_durations() {
        let mut i = inputs(vec![]);
        i.cooldown_lines = vec!["claude cooling for 12m".into(), "codex cooling for 3h 5m".into()];
        let r = diagnose(&i);
        let c = find(&r, "cooldowns");
        assert_eq!(c.severity, Severity::Info);
        assert!(c.detail.contains("claude cooling for 12m") && c.detail.contains("codex cooling for 3h 5m"));
        assert!(!has_check(&diagnose(&inputs(vec![])), "cooldowns"));
    }

    // ---- ordering and empties

    #[test]
    fn checks_are_ordered_problem_warn_info_ok() {
        use State::*;
        let mut i = inputs(all([Ready, NoKey, NotLoggedIn, NotInstalled, Ready]));
        i.jev_key_present = false;
        i.jev_reachable = None;
        i.in_git_repo = Some(false);
        i.cooldown_lines = vec!["claude cooling for 1m".into()];
        let r = diagnose(&i);
        let sev: Vec<Severity> = r.checks.iter().map(|c| c.severity).collect();
        let mut sorted = sev.clone();
        sorted.sort_by_key(|s| Reverse(*s));
        assert_eq!(sev, sorted);
        assert_eq!(r.checks[0].id, "jev-key");
        assert_eq!(r.checks.last().unwrap().severity, Severity::Ok);
        assert!(sev.contains(&Severity::Problem) && sev.contains(&Severity::Warn) && sev.contains(&Severity::Info));
    }

    #[test]
    fn empty_inputs_never_panic() {
        let r = diagnose(&Inputs::default());
        assert!(r.ready_providers.is_empty());
        assert!(!r.routing_note.is_empty());
        assert!(has_check(&r, "jev-key") && has_check(&r, "no-provider"));
        assert_eq!(first_run_hint(&[]).is_some(), true);
    }

    // ---- first run hint

    #[test]
    fn hint_is_absent_when_two_are_ready_and_nothing_is_half_configured() {
        use State::*;
        for states in [
            [Ready, NotInstalled, Ready, NotInstalled, NotInstalled],
            [Ready, NoKey, Ready, NotInstalled, NotInstalled],
            [Ready, Ready, Unknown, NotInstalled, NotInstalled],
            [Ready; 5],
        ] {
            assert_eq!(first_run_hint(&all(states)), None, "{states:?}");
        }
    }

    #[test]
    fn hint_names_a_half_configured_provider_even_with_two_ready() {
        use State::*;
        let hint = first_run_hint(&all([Ready, NotInstalled, Ready, NotLoggedIn, NotInstalled])).unwrap();
        assert!(hint.contains("gemini is installed but not logged in"), "{hint}");
        assert!(!hint.contains("nothing to route between"));
    }

    #[test]
    fn hint_with_one_ready_says_so_and_never_nags_about_uninstalled_providers() {
        use State::*;
        let hint = first_run_hint(&all([Ready, NotInstalled, NotInstalled, NotInstalled, NotInstalled])).unwrap();
        assert!(hint.contains("nothing to route between"));
        for name in ["codex", "gemini", "grok", "deepseek"] {
            assert!(!hint.contains(name), "{hint}");
        }
    }

    #[test]
    fn hint_gives_the_exact_fix_for_each_login() {
        use State::*;
        let hint = first_run_hint(&all([NotLoggedIn, NoKey, NotLoggedIn, NotInstalled, NotInstalled])).unwrap();
        assert!(hint.contains("claude is installed but not logged in: claude auth login"), "{hint}");
        assert!(hint.contains("codex is installed but not logged in: codex login"), "{hint}");
        assert!(hint.contains("deepseek has no API key: Add DEEPSEEK_API_KEY=<key> to ~/.jev-router.env"), "{hint}");
        assert!(hint.starts_with("No provider is ready yet"));
    }

    #[test]
    fn hint_with_nothing_installed_points_at_doctor() {
        let hint = first_run_hint(&all([State::NotInstalled; 5])).unwrap();
        assert!(hint.contains("No provider is ready") && hint.contains("doctor"), "{hint}");
    }

    #[test]
    fn deepseek_without_a_key_is_not_nagged_when_two_others_are_ready() {
        let hint = first_run_hint(&all([State::Ready, State::NoKey, State::Ready, State::NotInstalled, State::NotInstalled]));
        assert_eq!(hint, None);
    }

    #[test]
    fn block_reasons_are_worded_per_provider() {
        assert!(block_reason_text(Codex, BlockReason::UnusableHere).contains("git repository"));
        assert!(block_reason_text(Gemini, BlockReason::UnusableHere).contains("trusted workspace"));
        assert!(block_reason_text(Claude, BlockReason::Auth).contains("/providers refresh"));
    }

    #[test]
    fn login_commands_have_one_source_and_the_verified_ones_are_named() {
        assert_eq!(login_command(Claude), Some("claude auth login"));
        assert_eq!(login_command(Codex), Some("codex login"));
        assert_eq!(login_command(Gemini), Some("gemini"));
        assert_eq!(login_command(Grok), Some("grok"));
        assert_eq!(login_command(DeepSeek), None);
        for p in ALL_PROVIDERS {
            assert_eq!(login_verified(p), matches!(p, Claude | Codex), "{p}");
        }
    }

    #[test]
    fn doctor_steps_and_messages_use_the_same_login_command() {
        for p in ALL_PROVIDERS {
            let steps = login_steps(p, "~/.jev-router.env").join("\n");
            let sentence = login_instruction(p);
            match login_command(p) {
                Some(cmd) => {
                    assert!(steps.contains(cmd), "{p}: {steps}");
                    assert!(sentence.contains(&format!("`{cmd}`")), "{p}: {sentence}");
                    // The uifmt wording is built from the same sentence.
                    assert_eq!(crate::uifmt::next_steps_for(&crate::backend::ErrorKind::Auth, p.as_str())[0].to_lowercase(), sentence.to_lowercase());
                }
                None => {
                    assert!(steps.contains("DEEPSEEK_API_KEY") && sentence.contains("DEEPSEEK_API_KEY"));
                }
            }
            assert!(!install_hint(p).is_empty());
        }
    }
}
