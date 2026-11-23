//! Building an [`Engine`]: the real wiring and the `with_*` switches.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::domain::effort::EffortSetting;
use jev_changes::domain::turn_activity::TurnActivity;
use crate::application::guards::Guards;
use jev_approval::application::extension_hooks::Hooks;
use jev_session::domain::session::Sessions;
use jev_approval::domain::mode::Mode;
use jev_orchestration::application::ports::Checks;
use jev_orchestration::domain::orchestration_gate::Setup;
use jev_approval::domain::permission_rule;
use jev_providers::application::provider_registry::Runtime;
use jev_judgment::application::judge::Router;
use jev_routing::domain::decide::{Pin, PinOrigin, RoutingState};
use jev_routing::domain::sticky::Sticky;
use super::{Clock, Engine};
use jev_backend::BackendFactory;
use crate::application::ports::{Adapters, GitStatus};
use crate::application::usage_account::UsageAccount;

impl Engine {
    /// The given backends, git, turn log writer and idle timeout, the real clock, and the turn
    /// log path from `JEV_CODE_LOG`. Routing is on.
    pub fn new(runtime: Runtime, router: Router, dir: PathBuf, adapters: Adapters) -> Self {
        let host = adapters.host;
        let clock = host.clone();
        let mut guards = Guards { home: host.home(), ..Guards::default() };
        guards.set_fundings(runtime.fundings.clone());
        let turn_log = host.env("JEV_CODE_LOG").or_else(|| Some(host.state_dir()?.join("code-turns.jsonl").to_string_lossy().into_owned()));
        let allowed_tools = permission_rule::split_rules(&host.env("CODE_ALLOWED_TOOLS").unwrap_or_default());
        Self {
            runtime,
            sessions: Sessions::new(),
            routing: RoutingState::default(),
            guards,
            router,
            pin: Pin::Auto,
            pin_origin: PinOrigin::User,
            hooks: Arc::new(Hooks::none(&dir)),
            dir,
            enabled: true,
            backend_factory: adapters.backend_factory,
            git_status: adapters.git_status,
            now: Box::new(move || clock.now_ms()),
            host,
            path_facts: adapters.path_facts,
            remembered_rules: adapters.remembered_rules,
            privacy_audit: adapters.privacy_audit,
            usage_ledger: adapters.usage_ledger,
            liveness: adapters.liveness,
            session_store: adapters.sessions,
            idle_timeout: adapters.idle_timeout,
            stream_text: false,
            turn_log,
            turn_log_writer: adapters.turn_log,
            write_mcp: adapters.write_mcp,
            previous: None,
            last_context_tokens: 0,
            sticky: Sticky::default(),
            downgrade_after: 2,
            orchestrate: false,
            orch_setup: None,
            track_changes: true,
            working_tree: None,
            last_changes: None,
            change_history: Vec::new(),
            images: Vec::new(),
            turn_activity: TurnActivity::default(),
            persist: None,
            settings: None,
            git_snapshot: None,
            session_file: None,
            mode: Mode::default(),
            effort: EffortSetting::default(),
            allowed_tools,
            verify: false,
            checks: None,
        }
    }

    /// Keep a summary of what each turn changed, for `/diff` and `/undo` (on by default;
    /// it costs two `git` runs per turn and does nothing outside a repository).
    pub fn with_track_changes(mut self, on: bool) -> Self {
        self.track_changes = on;
        self
    }

    /// Save the session after every turn under `state_dir/sessions/` so it can be resumed
    /// after a restart. `None` (the default of the library) saves nothing; the binary
    /// passes the host's state directory.
    pub fn with_persistence(self, state_dir: Option<PathBuf>) -> Self {
        // The settings that fence in the router live beside the saved sessions.
        let settings = state_dir.clone();
        self.with_persistence_and_settings(state_dir, settings.as_deref())
    }

    /// Save sessions in `persist` (or not at all), but read the router's settings (privacy,
    /// budget, failover, routing rules) from `settings`: a one-shot keeps no session yet still
    /// obeys them.
    pub fn with_persistence_and_settings(mut self, persist: Option<PathBuf>, settings: Option<&std::path::Path>) -> Self {
        self.guards = Guards::load(&*self.host, self.privacy_audit.clone(), settings, &self.dir);
        self.guards.set_fundings(self.runtime.fundings.clone());
        if let Some(state) = settings {
            // The workers' processes see this session's spend, and this one theirs.
            let now = self.host.now_ms();
            let path = state.join(crate::application::ports::USAGE_LEDGER_FILE);
            self.guards = std::mem::take(&mut self.guards).with_ledger(UsageAccount::planner(self.usage_ledger.clone(), self.liveness.clone(), path, &self.dir, now));
        }
        self.persist = persist;
        self.settings = settings.map(Path::to_path_buf);
        self
    }

    /// Show assistant text the moment it arrives (see the field).
    pub fn with_streaming(mut self) -> Self {
        self.stream_text = true;
        self
    }

    /// Start in this mode (the binary reads `CODE_PERMISSION_MODE`).
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// Start with orchestration on or off (the binary reads `JEV_ORCHESTRATE`).
    pub fn with_orchestrate(mut self, on: bool) -> Self {
        self.orchestrate = on;
        self
    }

    /// Check each turn's changes with the project's own tests, run by `checks` (the binary
    /// reads `JEV_VERIFY`). Without `checks` nothing is verified.
    pub fn with_verify(mut self, on: bool, checks: Option<Arc<dyn Checks>>) -> Self {
        self.verify = on;
        self.checks = checks;
        self
    }

    /// Where the `delegate_mcp` binary and the MCP config directory are.
    pub fn with_orch_setup(mut self, setup: Option<Setup>) -> Self {
        self.orch_setup = setup;
        self
    }

    pub fn with_backend_factory(mut self, f: BackendFactory) -> Self {
        self.backend_factory = f;
        self
    }

    pub fn with_git_status(mut self, f: GitStatus) -> Self {
        self.git_status = f;
        self
    }

    pub fn with_clock(mut self, f: Clock) -> Self {
        self.now = f;
        self
    }

    /// The rules allowed without asking, in place of `CODE_ALLOWED_TOOLS`.
    pub fn with_allowed_tools(mut self, list: &str) -> Self {
        self.allowed_tools = permission_rule::split_rules(list);
        self
    }

    pub fn with_idle_timeout(mut self, d: Duration) -> Self {
        self.idle_timeout = d;
        self
    }

    pub fn with_turn_log(mut self, path: Option<String>) -> Self {
        self.turn_log = path;
        self
    }

    pub fn with_enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// Easier turns before the legacy path steps back down a tier.
    pub fn with_downgrade_after(mut self, n: u32) -> Self {
        self.downgrade_after = n;
        self
    }
}
