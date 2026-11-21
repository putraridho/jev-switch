//! Owns the provider facts of `/providers`, `/status`, the banner rows and Tab completion, read
//! from the runtime and the engine. Must not print or read the clock (`now_ms` is passed in).

use jev_backend::ProviderId;
use jev_catalog::domain::model_table as catalog;
use jev_providers::application::provider_registry::{BlockReason, Runtime};
use jev_providers::domain::availability::{Availability, State};
use jev_shared::domain::duration_text::format_left;
use jev_shared::ALL_PROVIDERS;
use jev_turn::application::engine::Engine;
use crate::render::{AvailRow, CompleteCtx, ProviderStatus, ReadyProvider, StatusFacts, SummaryFacts};
use super::startup::display_path;

fn status_of(state: State) -> ProviderStatus {
    match state {
        State::Ready => ProviderStatus::Ready,
        State::NotInstalled => ProviderStatus::NotInstalled,
        State::NotLoggedIn => ProviderStatus::NotLoggedIn,
        State::NoKey => ProviderStatus::NoKey,
        State::Unknown => ProviderStatus::Unknown,
    }
}

/// One row per provider as `/providers` and the banner show it: the probe, then what has
/// happened since (a usage-limit cooldown, a login that failed, a directory it refuses).
pub fn avail_rows(rt: &Runtime, now_ms: u64) -> Vec<AvailRow> {
    rt.avail.iter().map(|a| avail_row(a, rt, now_ms)).collect()
}

fn avail_row(a: &Availability, rt: &Runtime, now_ms: u64) -> AvailRow {
    let mut row = AvailRow::new(a.provider.as_str(), status_of(a.state));
    row.plan = a.plan.clone();
    row.models = a.models.clone();
    row.detail = a.detail.clone();
    row.verified = a.verified;
    row.cooldown = rt.cooldown_left(a.provider, now_ms).map(format_left);
    if a.provider == ProviderId::Gemini {
        row.via = Some(jev_providers::infrastructure::backend_factory::gemini_via().to_string());
    }
    match rt.blocked.get(&a.provider) {
        Some(BlockReason::Auth) => {
            row.status = ProviderStatus::NotLoggedIn;
            row.detail = "its login failed this session; log in again, then /providers refresh".into();
        }
        Some(BlockReason::UnusableHere) => {
            row.status = ProviderStatus::Unknown;
            row.detail = "can't run in this directory (/new clears it)".into();
        }
        Some(BlockReason::Unsupported) => {
            row.status = ProviderStatus::Unknown;
            row.detail = "its CLI is no longer supported by the service; see /providers".into();
        }
        None => {}
    }
    row
}

/// The models routing may pick right now, as `provider:model`, in catalog order.
fn usable_models(rt: &Runtime, now_ms: u64) -> Vec<(ProviderId, &'static str)> {
    let usable = rt.usable(now_ms);
    catalog::models().iter().filter(|m| usable.contains(m)).map(|m| (m.provider, m.id)).collect()
}

/// What Tab completion may offer: the providers and models that can run now, and the files
/// the last turn changed.
pub fn complete_ctx(engine: &Engine, now_ms: u64) -> CompleteCtx {
    let usable = usable_models(&engine.runtime, now_ms);
    let mut ready_providers: Vec<String> = Vec::new();
    for (p, _) in &usable {
        if !ready_providers.iter().any(|r| r == p.as_str()) {
            ready_providers.push(p.to_string());
        }
    }
    CompleteCtx {
        ready_providers,
        models: usable.iter().map(|(p, m)| format!("{p}:{m}")).collect(),
        changed_files: engine.last_changes().map(|c| c.files.iter().map(|f| f.path.clone()).collect()).unwrap_or_default(),
        skills: skill_choices(engine.dir()),
        mcp_servers: mcp_server_names(engine),
    }
}

/// Every configured MCP server once, for `/mcp login` completion.
fn mcp_server_names(engine: &Engine) -> Vec<String> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let rows = jev_backend::infrastructure::mcp_discovery::discover(engine.dir(), home.as_deref(), engine.orchestrate());
    jev_backend::infrastructure::mcp_server::group(&rows).into_iter().map(|s| s.name).collect()
}

/// The skills of this project and the user, for `/skill` completion.
fn skill_choices(project: &std::path::Path) -> Vec<crate::render::SkillChoice> {
    let lib = jev_approval::application::extension_library::Library::discover(&jev_approval::application::extensions::Places::from_env(project), &crate::render::command_names());
    lib.of_kind(jev_approval::application::extension_library::Kind::Skill)
        .map(|e| crate::render::SkillChoice { name: e.name.clone(), description: e.meta.description.clone(), hint: e.meta.hint.clone() })
        .collect()
}

/// The facts of `/status`.
pub fn status_facts(engine: &Engine, last_turn: Option<&SummaryFacts>, home: Option<&str>, now_ms: u64) -> StatusFacts {
    let rt = &engine.runtime;
    let usable = usable_models(rt, now_ms);
    let ready = ALL_PROVIDERS
        .iter()
        .filter(|p| usable.iter().any(|(u, _)| u == *p))
        .map(|p| ReadyProvider {
            provider: p.to_string(),
            plan: rt.avail.iter().find(|a| a.provider == *p).and_then(|a| a.plan.clone()),
            plan_used: rt.plan_utilization(*p),
            credit: rt.credit_left(*p),
        })
        .collect();
    let cooldowns = ALL_PROVIDERS
        .iter()
        .filter_map(|p| Some((p.to_string(), format_left(rt.cooldown_left(*p, now_ms)?))))
        .collect();
    StatusFacts {
        project: display_path(engine.dir(), home),
        routing_on: engine.enabled(),
        pin: engine.pin().to_string(),
        orchestrate: engine.orchestrate(),
        ready,
        cooldowns,
        last_turn: last_turn.cloned(),
        turns: engine.sessions.turns(),
    }
}
