//! Turning "which provider" into a runnable backend, and remembering what the
//! router has learned about providers while it runs (who is logged in, who hit a
//! limit, which models were refused).
//!
//! Choices are pure functions ([`choose_gemini_flavor`], [`deepseek_env`],
//! [`usable_from`], [`allowances_for`], [`format_left`]); the few IO calls (PATH,
//! settings file, environment) sit in thin wrappers beside them so the rules can be
//! tested without a CLI, a network or the user's home directory.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::availability::{self, Availability, State};
use crate::backend::claude::ClaudeBackend;
use crate::backend::codex::CodexBackend;
use crate::backend::gemini::{auth_looks_configured, Flavor, GeminiBackend, AUTH_ENV_VARS};
use crate::backend::grok::GrokBackend;
use crate::backend::{Backend, ErrorKind, PlanUsage, ProviderId, TurnError};
use crate::catalog::{self, Allowances, Overrides, Usable};
use crate::cooldown::{self, Cooldowns};
use crate::envfile;

/// Every provider, in the order `/providers` lists them.
pub const ALL_PROVIDERS: [ProviderId; 5] =
    [ProviderId::Claude, ProviderId::DeepSeek, ProviderId::Codex, ProviderId::Gemini, ProviderId::Grok];

const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/anthropic";
const MINUTE_MS: u64 = 60_000;

// ------------------------------------------------------------------ backends

/// Environment that points the `claude` CLI at DeepSeek's Anthropic-compatible API.
///
/// The key goes in both `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_API_KEY` because some
/// Claude Code versions read one and some the other.
///
/// UNVERIFIED: that `claude -p` with these variables bypasses the user's Claude
/// login and really talks to DeepSeek. The availability probe only checks that the
/// key exists, not that this wiring works.
pub fn deepseek_env(key: &str) -> Vec<(String, String)> {
    vec![
        ("ANTHROPIC_BASE_URL".to_string(), DEEPSEEK_BASE_URL.to_string()),
        ("ANTHROPIC_AUTH_TOKEN".to_string(), key.to_string()),
        ("ANTHROPIC_API_KEY".to_string(), key.to_string()),
    ]
}

/// Gemini CLI when it is installed and has auth; otherwise Antigravity. Gemini CLI
/// without auth exits immediately, so installed alone is not enough to prefer it.
pub fn choose_gemini_flavor(gemini_on_path: bool, gemini_configured: bool) -> Flavor {
    if gemini_on_path && gemini_configured {
        Flavor::GeminiCli
    } else {
        Flavor::Antigravity
    }
}

/// A fresh backend for one turn. Codex and Gemini keep per-turn parse state inside
/// the instance, so sharing one across turns would mix their output: build a new one
/// every time (construction is cheap).
pub fn backend_for(provider: ProviderId) -> Box<dyn Backend> {
    match provider {
        ProviderId::Claude => Box::new(ClaudeBackend::claude()),
        ProviderId::Codex => Box::new(CodexBackend::codex()),
        ProviderId::Grok => Box::new(GrokBackend::grok()),
        ProviderId::Gemini => Box::new(match detect_gemini_flavor() {
            Flavor::GeminiCli => GeminiBackend::gemini_cli(),
            Flavor::Antigravity => GeminiBackend::antigravity(),
        }),
        // No key means the caller never routes here; an empty key just fails the turn.
        ProviderId::DeepSeek => {
            let key = envfile::lookup("DEEPSEEK_API_KEY").unwrap_or_default();
            Box::new(ClaudeBackend::with_profile(ProviderId::DeepSeek, deepseek_env(&key)))
        }
    }
}

/// Gathers the two facts [`choose_gemini_flavor`] needs from the real machine.
fn detect_gemini_flavor() -> Flavor {
    let settings = std::env::var_os("HOME")
        .and_then(|h| std::fs::read_to_string(PathBuf::from(h).join(".gemini/settings.json")).ok());
    let env_has_key = AUTH_ENV_VARS.iter().any(|n| envfile::from_env(n).is_some());
    choose_gemini_flavor(
        crate::availability::on_path(&GeminiBackend::gemini_cli().bin),
        auth_looks_configured(settings.as_deref(), env_has_key),
    )
}

// ------------------------------------------------------------- pure decisions

/// Credits are opt-in via `JEV_ALLOW_CREDITS` ("1" or "true"). Per-token models are
/// allowed exactly when DeepSeek is Ready, because configuring its key is the
/// user's agreement to pay per token.
pub fn allowances_for(credits_env: Option<&str>, avail: &[Availability]) -> Allowances {
    Allowances {
        credits: credits_env.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true")),
        per_token: deepseek_ready(avail),
    }
}

fn deepseek_ready(avail: &[Availability]) -> bool {
    avail.iter().any(|a| a.provider == ProviderId::DeepSeek && a.state == State::Ready)
}

/// The models a turn may go to: those of Ready providers (narrowed by the login's
/// own model list), minus providers cooling down and models a provider refused.
pub fn usable_from(
    avail: &[Availability],
    cooldowns: &Cooldowns,
    unusable: &HashSet<(ProviderId, String)>,
    now_ms: u64,
) -> Usable {
    let catalog_ids = |p: ProviderId| {
        catalog::models().iter().filter(|m| m.provider == p).map(|m| m.id.to_string()).collect::<Vec<_>>()
    };
    Usable::new(
        availability::usable_models(avail, &catalog_ids)
            .into_iter()
            .filter(|(p, _)| cooldowns.active(*p, now_ms).is_none())
            .filter(|pair| !unusable.contains(pair)),
    )
}

/// "12m", "3h 5m", "2d 4h": short enough for one `/providers` line. Under a minute
/// reads "<1m" so a nearly finished cooldown is not shown as zero.
pub fn format_left(ms: u64) -> String {
    let minutes = ms / MINUTE_MS;
    match minutes {
        0 => "<1m".to_string(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, (minutes % 1440) / 60),
    }
}

// -------------------------------------------------------------------- Runtime

/// What the router knows about providers right now. Availability comes from probes;
/// cooldowns persist across runs; unusable models and plan utilization live only for
/// this run.
#[derive(Debug)]
pub struct Runtime {
    pub avail: Vec<Availability>,
    pub cooldowns: Cooldowns,
    pub allow: Allowances,
    pub overrides: Overrides,
    /// Where cooldowns are saved; `None` when there is no state directory.
    pub cooldown_path: Option<PathBuf>,
    /// Models a provider refused at run time (e.g. not entitled), so they are not retried.
    pub unusable_models: HashSet<(ProviderId, String)>,
    /// The latest plan utilization each provider reported.
    pub plan: HashMap<ProviderId, PlanUsage>,
    /// Providers that failed in a way another provider can cover and that is not a usage
    /// limit, with why. Session-only and kept out of the persisted cooldowns on purpose:
    /// a login problem or an unusable directory is not a limit, and an hour-long cooldown
    /// would only outlast a re-login. See [`BlockReason`] for what clears each.
    pub blocked: HashMap<ProviderId, BlockReason>,
}

/// Why a provider is skipped for the rest of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// The login failed. `/providers refresh` clears it once the probe sees a login.
    Auth,
    /// The CLI refuses to run in this directory (Codex outside a git repo, Gemini in an
    /// untrusted workspace). Probing cannot change that, so only `/new` or another
    /// project directory (a new session) clears it.
    UnusableHere,
}

/// What the user can do about [`BlockReason::UnusableHere`] for `provider`.
fn unusable_here_hint(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "needs a git repository here (git init, or work in a repo)",
        ProviderId::Gemini => "needs a trusted workspace here (run gemini in this directory once to trust it)",
        _ => "refuses to run in this directory",
    }
}

impl Runtime {
    /// Probe the machine and load persisted cooldowns.
    pub async fn start() -> Self {
        let avail = availability::probe_all().await;
        let cooldown_path = cooldown::default_path();
        let cooldowns = cooldown_path.as_deref().map(Cooldowns::load).unwrap_or_default();
        let allow = allowances_for(std::env::var("JEV_ALLOW_CREDITS").ok().as_deref(), &avail);
        Self::build(avail, cooldowns, allow, Overrides::from_env(), cooldown_path)
    }

    /// The one place a `Runtime` is assembled, so `start` and the tests share it.
    fn build(
        avail: Vec<Availability>,
        cooldowns: Cooldowns,
        allow: Allowances,
        overrides: Overrides,
        cooldown_path: Option<PathBuf>,
    ) -> Self {
        Self {
            avail,
            cooldowns,
            allow,
            overrides,
            cooldown_path,
            unusable_models: HashSet::new(),
            plan: HashMap::new(),
            blocked: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests(avail: Vec<Availability>) -> Self {
        let allow = allowances_for(None, &avail);
        Self::build(avail, Cooldowns::default(), allow, Overrides::default(), None)
    }

    /// The (provider, model) pairs routing may pick right now.
    pub fn usable(&self, now_ms: u64) -> Usable {
        if self.blocked.is_empty() {
            return usable_from(&self.avail, &self.cooldowns, &self.unusable_models, now_ms);
        }
        let unblocked: Vec<Availability> =
            self.avail.iter().filter(|a| !self.blocked.contains_key(&a.provider)).cloned().collect();
        usable_from(&unblocked, &self.cooldowns, &self.unusable_models, now_ms)
    }

    /// Skip `p` until its limit resets, and remember that across runs. A failed save
    /// costs only the persistence, so it is reported and otherwise ignored.
    pub fn cool_down(&mut self, p: ProviderId, resets_at: Option<&str>, now_ms: u64) {
        self.cooldowns.start(p, resets_at, now_ms);
        let Some(path) = &self.cooldown_path else { return };
        if let Err(e) = self.cooldowns.save(path) {
            eprintln!("could not save cooldowns to {}: {e}", path.display());
        }
    }

    /// Stop offering `model` of `p` for the rest of this run.
    pub fn mark_unusable(&mut self, p: ProviderId, model: &str) {
        self.unusable_models.insert((p, model.to_string()));
    }

    pub fn note_plan(&mut self, p: ProviderId, plan: PlanUsage) {
        self.plan.insert(p, plan);
    }

    /// Fraction of the plan window used (as the CLI reported it), if any turn has said.
    pub fn plan_utilization(&self, p: ProviderId) -> Option<f64> {
        self.plan.get(&p).map(|u| u.utilization)
    }

    /// Apply what a failed attempt teaches the router and say what happens next. `Some`
    /// is the note for "try the next model"; `None` means the failure would likely recur
    /// on every provider, so the caller stops. Shared by a turn's failover
    /// ([`crate::turn::Engine`]) and by orchestration workers
    /// ([`crate::delegate::Delegator`]) so a limit, a login or an entitlement is handled
    /// one way everywhere.
    pub fn react(
        &mut self,
        provider: ProviderId,
        model: &str,
        err: &TurnError,
        excluded: &mut HashSet<(ProviderId, String)>,
        now: u64,
    ) -> Option<String> {
        match &err.kind {
            ErrorKind::Quota { resets_at } => {
                self.cool_down(provider, resets_at.as_deref(), now);
                let when = resets_at.as_deref().map(|r| format!(" (resets {r})")).unwrap_or_default();
                Some(format!("{provider} hit its limit{when}: trying the next model"))
            }
            ErrorKind::NotEntitled => {
                self.mark_unusable(provider, model);
                Some(format!("{provider}:{model} is not on your plan: trying the next model"))
            }
            ErrorKind::Auth => {
                self.blocked.insert(provider, BlockReason::Auth);
                Some(format!(
                    "{provider} is not logged in or its login expired (log in to {} again, then /providers refresh): trying the next model",
                    login_target(provider)
                ))
            }
            ErrorKind::UnusableHere => {
                self.blocked.insert(provider, BlockReason::UnusableHere);
                Some(format!("{provider} {}: trying the next model", unusable_here_hint(provider)))
            }
            ErrorKind::Stalled | ErrorKind::Empty => {
                excluded.insert((provider, model.to_string()));
                let what = if err.kind == ErrorKind::Stalled { "stopped answering" } else { "gave an empty answer" };
                Some(format!("{provider}:{model} {what}: trying the next model"))
            }
            ErrorKind::Other => None,
        }
    }

    /// Probe again (for `/providers refresh`). Cooldowns, unusable models and plan
    /// readings are kept: a re-login does not end a usage limit. A provider that failed
    /// its login is offered again once the probe sees it logged in. The per-token
    /// allowance is recomputed because a key may have been added.
    pub async fn refresh(&mut self) {
        self.apply_probe(availability::probe_all().await);
    }

    /// The pure half of [`Runtime::refresh`].
    fn apply_probe(&mut self, avail: Vec<Availability>) {
        self.avail = avail;
        self.allow.per_token = deepseek_ready(&self.avail);
        let avail = &self.avail;
        self.blocked.retain(|p, reason| {
            *reason != BlockReason::Auth || !avail.iter().any(|a| a.provider == *p && a.state == State::Ready)
        });
    }

    /// `/new`: the directory-dependent blocks end with the session they were learned in.
    pub fn forget_unusable_here(&mut self) {
        self.blocked.retain(|_, reason| *reason != BlockReason::UnusableHere);
    }

    /// The text of `/providers`: probe results, then what has changed since.
    pub fn render(&self, now_ms: u64) -> String {
        let mut out = vec![availability::render(&self.avail)];
        out.extend(self.cooldown_lines(now_ms));
        out.extend(self.unusable_lines());
        out.extend(self.blocked_lines());
        out.push(format!(
            "allowed: credits {}, per-token {}",
            yes_no(self.allow.credits),
            yes_no(self.allow.per_token)
        ));
        out.extend(self.overrides.notes.iter().map(|n| format!("override: {n}")));
        out.join("\n")
    }

    /// The compact startup view: only providers that are ready, any cooldowns carried
    /// over from earlier runs, and where to find the rest.
    pub fn render_summary(&self, now_ms: u64) -> String {
        let mut lines: Vec<String> = Vec::new();
        let ready = availability::render_ready(&self.avail);
        lines.push(if ready.is_empty() { "no provider is ready".to_string() } else { ready });
        lines.extend(self.cooldown_lines(now_ms));
        lines.push("/providers for details".to_string());
        lines.join("\n")
    }

    fn cooldown_lines(&self, now_ms: u64) -> Vec<String> {
        ALL_PROVIDERS
            .iter()
            .filter_map(|p| Some(format!("{p} cooling for {}", format_left(self.cooldowns.active(*p, now_ms)?))))
            .collect()
    }

    /// One line per provider blocked for this session, with why and what clears it.
    fn blocked_lines(&self) -> Vec<String> {
        ALL_PROVIDERS
            .iter()
            .filter_map(|p| {
                let line = match self.blocked.get(p)? {
                    BlockReason::Auth => format!(
                        "{p} blocked this session: not logged in (log in to {}, then /providers refresh)",
                        login_target(*p)
                    ),
                    BlockReason::UnusableHere => {
                        format!("{p} blocked this session: {} (/new clears it)", unusable_here_hint(*p))
                    }
                };
                Some(line)
            })
            .collect()
    }

    fn unusable_lines(&self) -> Vec<String> {
        let mut models: Vec<_> = self.unusable_models.iter().collect();
        models.sort();
        models.into_iter().map(|(p, m)| format!("{p}:{m} refused by the provider, not offered")).collect()
    }
}

/// The tool a user logs in to again for `provider`.
pub fn login_target(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "claude",
        ProviderId::Codex => "codex",
        ProviderId::Gemini => "gemini (or agy)",
        ProviderId::Grok => "grok",
        ProviderId::DeepSeek => "DeepSeek: check DEEPSEEK_API_KEY",
    }
}

fn yes_no(v: bool) -> &'static str {
    if v {
        "yes"
    } else {
        "no"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProviderId::{Claude, Codex, DeepSeek, Gemini};

    const NOW: u64 = 1_760_000_000_000;

    fn availability(provider: ProviderId, state: State) -> Availability {
        Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
    }

    fn some_model(p: ProviderId) -> &'static catalog::ModelSpec {
        catalog::models().iter().find(|m| m.provider == p).expect("catalog has the provider")
    }

    fn runtime() -> Runtime {
        Runtime::for_tests(vec![
            availability(Claude, State::Ready),
            availability(Codex, State::Ready),
            availability(Gemini, State::NotLoggedIn),
            availability(DeepSeek, State::NoKey),
        ])
    }

    #[test]
    fn usable_has_ready_providers_only() {
        let u = runtime().usable(NOW);
        assert!(u.contains(some_model(Claude)));
        assert!(u.contains(some_model(Codex)));
        assert!(!u.contains(some_model(Gemini)));
        assert!(!u.contains(some_model(DeepSeek)));
    }

    #[test]
    fn usable_excludes_cooling_provider_until_it_resets() {
        let mut rt = runtime();
        rt.cooldowns.start(Claude, None, NOW);
        assert!(!rt.usable(NOW + 1).contains(some_model(Claude)));
        assert!(rt.usable(NOW + 1).contains(some_model(Codex)));
        // Default cooldown is an hour; after that the provider is back.
        assert!(rt.usable(NOW + 2 * 60 * MINUTE_MS).contains(some_model(Claude)));
    }

    #[test]
    fn usable_excludes_a_refused_model_but_not_its_siblings() {
        let mut rt = runtime();
        let claude: Vec<_> = catalog::models().iter().filter(|m| m.provider == Claude).collect();
        assert!(claude.len() >= 2, "test needs two Claude models");
        rt.mark_unusable(Claude, claude[0].id);
        let u = rt.usable(NOW);
        assert!(!u.contains(claude[0]));
        assert!(u.contains(claude[1]));
    }

    #[test]
    fn login_model_list_narrows_usable() {
        let mut a = availability(Claude, State::Ready);
        let claude: Vec<_> = catalog::models().iter().filter(|m| m.provider == Claude).collect();
        a.models = Some(vec![claude[0].id.to_string()]);
        let u = Runtime::for_tests(vec![a]).usable(NOW);
        assert!(u.contains(claude[0]));
        assert!(!u.contains(claude[1]));
    }

    #[test]
    fn allowances_follow_credits_env_and_deepseek_key() {
        let ready = [availability(DeepSeek, State::Ready)];
        let no_key = [availability(DeepSeek, State::NoKey)];
        assert_eq!(allowances_for(None, &ready), Allowances { credits: false, per_token: true });
        assert_eq!(allowances_for(Some("1"), &no_key), Allowances { credits: true, per_token: false });
        assert!(allowances_for(Some("true"), &[]).credits);
        assert!(allowances_for(Some(" TRUE "), &[]).credits);
        assert!(!allowances_for(Some("0"), &[]).credits);
        assert!(!allowances_for(Some("yes"), &[]).credits);
        assert!(!allowances_for(Some(""), &[]).credits);
    }

    #[test]
    fn render_mentions_cooldown_refusals_allowances_and_overrides() {
        let mut rt = runtime();
        rt.cooldowns.start(Claude, None, NOW);
        rt.mark_unusable(Codex, "some-model");
        rt.overrides.notes.push("JEV_CHAINS ignored: boom".to_string());
        let text = rt.render(NOW + 12 * MINUTE_MS);
        assert!(text.contains("claude cooling for 48m"), "{text}");
        assert!(!text.contains("codex cooling"), "{text}");
        assert!(text.contains("codex:some-model refused"), "{text}");
        assert!(text.contains("credits no, per-token no"), "{text}");
        assert!(text.contains("override: JEV_CHAINS ignored: boom"), "{text}");
        assert!(text.contains(&availability::render(&rt.avail)), "{text}");
    }

    #[test]
    fn render_without_events_has_no_cooldown_lines() {
        assert!(!runtime().render(NOW).contains("cooling"));
    }

    #[test]
    fn format_left_picks_a_readable_unit() {
        assert_eq!(format_left(0), "<1m");
        assert_eq!(format_left(59_999), "<1m");
        assert_eq!(format_left(12 * MINUTE_MS), "12m");
        assert_eq!(format_left(125 * MINUTE_MS), "2h 5m");
        assert_eq!(format_left((2 * 1440 + 4 * 60) * MINUTE_MS), "2d 4h");
    }

    #[test]
    fn plan_utilization_reports_the_latest_reading() {
        let mut rt = runtime();
        assert_eq!(rt.plan_utilization(Claude), None);
        rt.note_plan(Claude, PlanUsage { window: "5h".into(), utilization: 0.4, resets_at: None });
        rt.note_plan(Claude, PlanUsage { window: "5h".into(), utilization: 0.7, resets_at: None });
        assert_eq!(rt.plan_utilization(Claude), Some(0.7));
        assert_eq!(rt.plan_utilization(Codex), None);
    }

    #[test]
    fn cool_down_persists_to_the_given_path() {
        let path = std::env::temp_dir().join(format!("jev-providers-test-{}.json", std::process::id()));
        let mut rt = runtime();
        rt.cooldown_path = Some(path.clone());
        rt.cool_down(Codex, Some("1760003600"), NOW);
        let loaded = Cooldowns::load(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(loaded.active(Codex, NOW), Some(3_600_000));
        assert!(rt.cooldowns.active(Codex, NOW).is_some());
    }

    #[test]
    fn cool_down_without_a_path_still_cools() {
        let mut rt = runtime();
        rt.cool_down(Claude, None, NOW);
        assert!(rt.cooldowns.active(Claude, NOW).is_some());
    }

    #[test]
    fn gemini_cli_needs_both_binary_and_auth() {
        assert_eq!(choose_gemini_flavor(true, true), Flavor::GeminiCli);
        assert_eq!(choose_gemini_flavor(true, false), Flavor::Antigravity);
        assert_eq!(choose_gemini_flavor(false, true), Flavor::Antigravity);
        assert_eq!(choose_gemini_flavor(false, false), Flavor::Antigravity);
    }

    #[test]
    fn deepseek_env_points_at_deepseek_with_the_key_twice() {
        let env = deepseek_env("sk-test");
        assert!(env.contains(&("ANTHROPIC_BASE_URL".to_string(), "https://api.deepseek.com/anthropic".to_string())));
        assert!(env.contains(&("ANTHROPIC_AUTH_TOKEN".to_string(), "sk-test".to_string())));
        assert!(env.contains(&("ANTHROPIC_API_KEY".to_string(), "sk-test".to_string())));
        assert_eq!(env.len(), 3);
    }

    #[test]
    fn a_directory_refusal_blocks_the_provider_with_a_reason_until_new_session() {
        let ready = |p| Availability { provider: p, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true };
        let mut rt = Runtime::for_tests(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        let mut excluded = HashSet::new();
        let err = TurnError { kind: ErrorKind::UnusableHere, message: "Not inside a trusted directory".into() };
        let note = rt.react(ProviderId::Codex, "gpt-6-luna", &err, &mut excluded, 1_000).unwrap();
        assert!(note.contains("codex needs a git repository here") && note.contains("trying the next model"), "{note}");
        assert_eq!(rt.blocked.get(&ProviderId::Codex), Some(&BlockReason::UnusableHere));
        assert!(rt.cooldowns.active(ProviderId::Codex, 1_000).is_none(), "not a usage limit");
        assert!(!rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Claude, "claude-sonnet-5-5").unwrap()));
        assert!(rt.render(1_000).contains("codex blocked this session: needs a git repository here"), "{}", rt.render(1_000));
        // A probe that sees codex logged in does not change the directory ...
        rt.apply_probe(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        assert_eq!(rt.blocked.get(&ProviderId::Codex), Some(&BlockReason::UnusableHere));
        // ... but a login block still clears while this one stays; /new clears this one.
        rt.blocked.insert(ProviderId::Claude, BlockReason::Auth);
        rt.apply_probe(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        assert_eq!(rt.blocked.len(), 1);
        rt.forget_unusable_here();
        assert!(rt.blocked.is_empty());
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
    }

    #[test]
    fn a_failed_login_is_session_only_and_clears_when_the_probe_sees_a_login() {
        let ready = |p| Availability { provider: p, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true };
        let mut rt = Runtime::for_tests(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        let mut excluded = HashSet::new();
        let err = TurnError { kind: ErrorKind::Auth, message: "401".into() };
        let note = rt.react(ProviderId::Codex, "gpt-6-luna", &err, &mut excluded, 1_000).unwrap();
        assert!(note.contains("log in"), "{note}");
        // not in the persisted cooldowns, but not offered either
        assert!(rt.cooldowns.active(ProviderId::Codex, 1_000).is_none());
        assert!(!rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Claude, "claude-sonnet-5-5").unwrap()));
        // a probe that still says "not logged in" keeps it blocked ...
        let mut not_logged = ready(ProviderId::Codex);
        not_logged.state = State::NotLoggedIn;
        rt.apply_probe(vec![ready(ProviderId::Claude), not_logged]);
        assert_eq!(rt.blocked.get(&ProviderId::Codex), Some(&BlockReason::Auth));
        // ... and one that sees the login again offers it right away
        rt.apply_probe(vec![ready(ProviderId::Claude), ready(ProviderId::Codex)]);
        assert!(rt.blocked.is_empty());
        assert!(rt.usable(1_000).contains(catalog::find(ProviderId::Codex, "gpt-6-luna").unwrap()));
    }
}
