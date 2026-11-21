//! Owns how a run is set up: the Jev client, the provider probe and the engine. Must not run a turn.

use std::io::IsTerminal;
use std::path::PathBuf;
use crate::tui::cli_args::Args;
use crate::tui::command as commands;
use crate::tui::console::terminal_ui::Console;
use crate::shared::infrastructure::env_file as envfile;
use crate::shared::infrastructure::jsonl_log as log;
use crate::approval::domain::mode;
use crate::orchestration::domain::orchestration_gate as orchestrate;
use crate::orchestration::domain::orchestration_gate::Setup;
use crate::tui::app::preferences::Saved;
use crate::providers::domain::probe_source::ProbeSource;
use crate::providers::infrastructure::probe_cache as probecache;
use crate::providers::application::provider_registry::{ProbeCache, Runtime};
use crate::judgment::application::judge::Router;
use crate::judgment::domain::thresholds::CODE_STRONG_THRESHOLD;
use crate::turn::application::engine::Engine;
use crate::judgment::infrastructure::typesafe_http::TypeSafe;
use crate::tui::render as uifmt;
use crate::tui::render::Style;

// ------------------------------------------------------------------------ setup

/// The Jev client, or the message that says how to get a key.
pub(super) fn jev_client(style: Style) -> Result<TypeSafe, String> {
    TypeSafe::from_env().map_err(|_| {
        let steps = [
            "Add JEV_API_KEY=... to ~/.jev-router.env (copy .jev-router.env.example and `chmod 600` it)".to_string(),
            "Or export JEV_API_KEY (or TYPESAFE_API_KEY) in your shell".to_string(),
            "Then check everything with `jev-code doctor`".to_string(),
        ];
        uifmt::problem_block("No Jev key found", &["jev-code asks Jev to judge each task, so it needs a key.".to_string()], &steps, style)
    })
}

/// Whether the provider probe may reuse the last run's answer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ProbeMode {
    /// Sessions and one-shot turns: a fresh cached answer is used at once and checked in the
    /// background, so the prompt never waits for the provider CLIs.
    Cached,
    /// `doctor` reports what is true now.
    Live,
}

/// The probe cache of this machine, unless the user turned it off.
pub(super) fn probe_cache() -> Option<ProbeCache> {
    if probecache::disabled(envfile::lookup("JEV_PROBE_CACHE").as_deref()) {
        return None;
    }
    Some(ProbeCache { path: probecache::default_path()?, fingerprint: probecache::fingerprint(&|name| envfile::lookup(name)) })
}

/// Probe the providers, with a one-line notice on a terminal when it has to ask the CLIs
/// (that can take seconds); a cached answer needs no notice.
pub(super) async fn start_runtime(console: &Console, mode: ProbeMode) -> (Runtime, ProbeSource) {
    let cache = (mode == ProbeMode::Cached).then(probe_cache).flatten();
    let warm = cache.as_ref().is_some_and(|c| probecache::load(&c.path, log::now_ms(), c.fingerprint).is_some());
    let live = !warm && console.err_style().enabled && std::io::stderr().is_terminal();
    if live {
        eprint!("{}", console.err_style().dim("checking providers..."));
    }
    let started = Runtime::start_cached(cache, std::sync::Arc::new(crate::providers::infrastructure::probe_facts::SystemProbes)).await;
    if live {
        eprint!("{}", uifmt::clear_line());
    }
    started
}

/// Settings come from the environment or `~/.jev-router.env`, like every other setting.
/// `can_ask`: the console can put an approval question to a person
/// ([`Console::can_ask`]); otherwise the CLIs run without the host prompt protocol.
/// `session`: what the last session left (`/mode`, `/orchestrate`, `/provider`) for an
/// interactive session; a one-shot run nobody can answer passes `None` and starts from the
/// flags and the environment alone.
pub(super) fn engine_for(args: &Args, runtime: Runtime, router: Router, dir: PathBuf, can_ask: bool, session: Option<Saved>) -> Engine {
    let saved = session.unwrap_or_default();
    let mode = mode::start(args.mode, saved.mode, envfile::lookup("CODE_PERMISSION_MODE").as_deref());
    let routing = saved.routing.unwrap_or_else(|| commands::typo::routing_enabled(envfile::lookup("JEV_PROVIDERS").as_deref()));
    let orchestrate = orchestrate::start(saved.orchestrate, envfile::lookup("JEV_ORCHESTRATE").as_deref(), session.is_some());
    let hooks = std::sync::Arc::new(crate::approval::application::extension_hooks::Hooks::load(&crate::approval::application::extensions::Places::from_env(&dir)));
    let engine = Engine::new(runtime, router, dir)
        .with_working_tree(std::sync::Arc::new(crate::changes::infrastructure::git_tree::GitWorkingTree))
        .with_backend_factory(Box::new(crate::providers::infrastructure::backend_factory::run_backends(can_ask)))
        .with_enabled(routing)
        .with_mode(mode)
        .with_orchestrate(orchestrate)
        .with_orch_setup(Setup::detect())
        .with_hooks(hooks)
        .with_downgrade_after(envfile::lookup_f64("DOWNGRADE_AFTER", 2.0) as u32);
    match &args.pin {
        Some(pin) => {
            let mut engine = engine;
            engine.set_pin(pin.clone());
            engine
        }
        None => engine,
    }
}

pub(super) fn router_for(ts: TypeSafe) -> Router {
    // The strong threshold only matters with routing off (the Claude-only tier rule).
    Router::new(ts, 1.0).with_strong_threshold(envfile::lookup_f64("CODE_STRONG_THRESHOLD", CODE_STRONG_THRESHOLD))
}
