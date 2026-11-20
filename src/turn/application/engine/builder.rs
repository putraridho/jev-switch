//! Building an [`Engine`]: the real wiring and the `with_*` switches.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::turn::domain::effort::EffortSetting;
use crate::changes::domain::turn_activity::TurnActivity;
use crate::turn::application::guards::Guards;
use crate::approval::application::extension_hooks::Hooks;
use crate::session::domain::catch_up::Sessions;
use crate::shared::infrastructure::jsonl_log as log;
use crate::approval::domain::mode::Mode;
use crate::orchestration::domain::orchestration_gate::Setup;
use crate::approval::domain::permission_rule;
use crate::providers::application::provider_registry::Runtime;
use crate::judgment::application::judge::Router;
use crate::routing::domain::decide::{Pin, PinOrigin, RoutingState, Sticky};
use super::{Clock, Engine, GitStatus};
use crate::backend::{idle_timeout_from_env};
use crate::turn::infrastructure::git_status::git_status_short;
use crate::backend::BackendFactory;

impl Engine {
    /// Real backends, the real clock and git, and the turn log from `JEV_CODE_LOG`.
    /// Routing is on.
    pub fn new(runtime: Runtime, router: Router, dir: PathBuf) -> Self {
        Self {
            runtime,
            sessions: Sessions::new(),
            routing: RoutingState::default(),
            guards: Guards::default(),
            router,
            pin: Pin::Auto,
            pin_origin: PinOrigin::User,
            hooks: Arc::new(Hooks::none(&dir)),
            dir,
            enabled: true,
            backend_factory: Box::new(crate::providers::infrastructure::backend_factory::session_backend_for),
            git_status: Box::new(git_status_short),
            now: Box::new(log::now_ms),
            idle_timeout: idle_timeout_from_env(),
            stream_text: false,
            turn_log: log::path_from_env("JEV_CODE_LOG", "code-turns.jsonl"),
            previous: None,
            last_context_tokens: 0,
            sticky: Sticky::default(),
            downgrade_after: 2,
            orchestrate: false,
            orch_setup: None,
            track_changes: true,
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
            allowed_tools: permission_rule::split_rules(&crate::shared::infrastructure::env_file::lookup("CODE_ALLOWED_TOOLS").unwrap_or_default()),
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
    /// passes [`log::state_dir`].
    pub fn with_persistence(self, state_dir: Option<PathBuf>) -> Self {
        // The settings that fence in the router live beside the saved sessions.
        let settings = state_dir.clone();
        self.with_persistence_and_settings(state_dir, settings.as_deref())
    }

    /// Save sessions in `persist` (or not at all), but read the router's settings (privacy,
    /// budget, failover, routing rules) from `settings`: a one-shot keeps no session yet still
    /// obeys them.
    pub fn with_persistence_and_settings(mut self, persist: Option<PathBuf>, settings: Option<&std::path::Path>) -> Self {
        self.guards = Guards::load(settings, &self.dir);
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
