//! Desktop-app composition for [`Engine`]: probes providers, attaches the workspace
//! directory and builds a session-capable, interactive [`Engine`] for Flashboard.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use jev_approval::application::extension_hooks::Hooks;
use jev_approval::application::extensions::Places;
use jev_approval::domain::mode::Mode;
use jev_catalog::domain::model_table;
use jev_changes::infrastructure::git_tree::GitWorkingTree;
use jev_judgment::application::judge::Router;
use jev_judgment::domain::thresholds::CODE_STRONG_THRESHOLD;
use jev_judgment::infrastructure::typesafe_http::TypeSafe;
use jev_orchestration::domain::orchestration_gate::{self, Setup};
use jev_orchestration::infrastructure::worker_check::ProjectChecks;
use jev_providers::application::provider_registry::{ProbeCache, Runtime};
use jev_providers::domain::availability::State as ProviderState;
use jev_providers::infrastructure::backend_factory::run_backends;
use jev_providers::infrastructure::probe_cache as probecache;
use jev_providers::infrastructure::probe_facts::SystemProbes;
use jev_routing::domain::decide::Pin;
use jev_shared::infrastructure::env_file as envfile;
use jev_shared::infrastructure::jsonl_log as log;
use jev_shared::ProviderId;

use crate::application::engine::Engine;
use crate::domain::outcomes::Outcomes;
use crate::domain::verify_mode::VerifyMode;
use crate::infrastructure::{audit, engine_adapters};

const VERIFY_TIMEOUT: Duration = Duration::from_secs(300);

/// Result of assembling a desktop [`Engine`].
pub struct DesktopEngine {
    /// The configured engine ready to run turns in `dir`.
    pub engine: Engine,
    /// Whether `JEV_API_KEY` / `TYPESAFE_API_KEY` was found for smart task judgment.
    pub jev_key_present: bool,
}

/// Summary of one provider's readiness in the desktop chat session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderReadiness {
    pub provider: ProviderId,
    pub ready: bool,
    pub detail: String,
    pub plan: Option<String>,
}

/// Build an interactive, streaming [`Engine`] rooted at `dir`.
pub async fn build_desktop_engine(
    dir: PathBuf,
    mode: Mode,
    orchestrate: bool,
    pin: Pin,
    routing_enabled: bool,
) -> DesktopEngine {
    let cache = if probecache::disabled(envfile::lookup("JEV_PROBE_CACHE").as_deref()) {
        None
    } else {
        probecache::default_path().map(|path| ProbeCache {
            path,
            fingerprint: probecache::fingerprint(&|name| envfile::lookup(name)),
        })
    };
    let (mut runtime, _) = Runtime::start_cached(cache, Arc::new(SystemProbes)).await;
    runtime.attach_project(&dir);

    let (ts, jev_key_present) = match TypeSafe::from_env() {
        Ok(client) => (client, true),
        Err(_) => (TypeSafe::unconfigured(), false),
    };
    let router = Router::new(ts, 1.0)
        .with_jev_timeout_from(&jev_shared::infrastructure::system_host::SystemHost)
        .with_strong_threshold(envfile::lookup_f64("CODE_STRONG_THRESHOLD", CODE_STRONG_THRESHOLD));

    let hooks = Arc::new(Hooks::load(&Places::from_env(&dir)));
    let checks = Arc::new(ProjectChecks { timeout: VERIFY_TIMEOUT });
    let state = log::state_dir();
    let entries = state
        .as_deref()
        .map(|d| audit::read(&audit::path_in(d)))
        .unwrap_or_default();
    let learn = orchestration_gate::env_choice(envfile::lookup("JEV_LEARNED_ROUTING").as_deref())
        != Some(false);
    let orch_setup = Setup::detect(
        state.clone(),
        envfile::lookup("JEV_DELEGATE_BIN"),
        std::env::current_exe().ok().as_deref(),
        &|p| p.is_file(),
    );
    let downgrade_after = envfile::lookup_f64("DOWNGRADE_AFTER", 2.0).max(0.0) as u32;

    let mut engine = Engine::new(runtime, router, dir, engine_adapters::real())
        .with_working_tree(Arc::new(GitWorkingTree))
        .with_backend_factory(Box::new(run_backends(true)))
        .with_streaming()
        .with_enabled(routing_enabled)
        .with_mode(mode)
        .with_orchestrate(orchestrate)
        .with_verify(
            VerifyMode::from_env(envfile::lookup("JEV_VERIFY").as_deref()),
            Some(checks.clone()),
            Some(checks),
        )
        .with_outcomes(Outcomes::from_entries(&entries), learn)
        .with_orch_setup(orch_setup)
        .with_hooks(hooks)
        .with_downgrade_after(downgrade_after)
        .with_persistence_and_settings(state.clone(), state.as_deref());
    engine.set_pin(pin);

    DesktopEngine { engine, jev_key_present }
}

/// Snapshot of provider readiness known by `engine`.
pub fn provider_readiness(engine: &Engine) -> Vec<ProviderReadiness> {
    engine
        .runtime
        .avail
        .iter()
        .map(|a| ProviderReadiness {
            provider: a.provider,
            ready: a.state == ProviderState::Ready,
            detail: a.detail.clone(),
            plan: a.plan.clone(),
        })
        .collect()
}

/// All catalog model `(provider, id)` pairs for UI selection.
pub fn catalog_models() -> Vec<(ProviderId, &'static str)> {
    model_table::models()
        .iter()
        .map(|spec| (spec.provider, spec.id))
        .collect()
}
