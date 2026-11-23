//! What an [`Engine`] shows and lets a command change in its own state.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use jev_routing::domain::warm_cache::WarmCache;
use crate::domain::effort::EffortSetting;
use jev_approval::application::extension_hooks::Hooks;
use jev_approval::domain::mode::Mode;
use jev_routing::domain::decide::{Pin, PinOrigin};
use crate::domain::outcomes::Outcomes;
use crate::domain::verify_mode::VerifyMode;
use super::Engine;

impl Engine {
    /// The project's places for judging grants: its folder, the home and temp folders, and
    /// the disk they are on.
    pub(super) fn places(&self) -> jev_approval::domain::grantable::Places {
        jev_approval::domain::grantable::Places {
            project: self.dir.clone(),
            home: self.host.home().map(std::path::PathBuf::from),
            temp: self.host.temp_dir(),
            disk: jev_approval::domain::grantable::Disk(self.path_facts.clone()),
        }
    }

    /// The guards and the folder their settings are saved in: what `/budget`, `/privacy`,
    /// `/failover` and `/health` act on.
    pub fn guards_and_settings(&mut self) -> (&mut crate::application::guards::Guards, Option<&Path>) {
        (&mut self.guards, self.settings.as_deref())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Hooks for tool calls and turn events.
    pub fn with_hooks(mut self, hooks: Arc<Hooks>) -> Self {
        self.hooks = hooks;
        self
    }

    pub fn set_hooks(&mut self, hooks: Arc<Hooks>) {
        self.hooks = hooks;
    }

    pub fn hooks(&self) -> &Arc<Hooks> {
        &self.hooks
    }

    pub fn pin(&self) -> &Pin {
        &self.pin
    }

    /// The last answer the session got (resumed sessions included), for the next-prompt
    /// suggestion.
    pub fn last_answer(&self) -> Option<&str> {
        self.previous.as_ref().map(|(_, reply)| reply.as_str())
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The person's own pin: routing rules step aside for it.
    pub fn set_pin(&mut self, pin: Pin) {
        self.pin = pin;
        self.pin_origin = PinOrigin::User;
    }

    /// A pin that came from the repo's team file: the person's routing rules still apply.
    pub fn set_team_pin(&mut self, pin: Pin) {
        self.pin = pin;
        self.pin_origin = PinOrigin::Team;
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Applies from the next turn on; a turn already running keeps the mode it started in.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub fn effort(&self) -> EffortSetting {
        self.effort
    }

    /// Applies from the next turn on.
    pub fn set_effort(&mut self, effort: EffortSetting) {
        self.effort = effort;
    }

    /// The prefix the last answering provider has cached, for `/cache` and the footer.
    pub fn warm_cache(&self) -> Option<WarmCache> {
        self.routing.warm
    }

    pub fn orchestrate(&self) -> bool {
        self.orchestrate
    }

    pub fn set_orchestrate(&mut self, on: bool) {
        self.orchestrate = on;
    }

    pub fn verify_mode(&self) -> &VerifyMode {
        &self.quality.verify
    }

    pub fn set_verify_mode(&mut self, mode: VerifyMode) {
        self.quality.verify = mode;
    }

    /// What past turns came to, per model and task type.
    pub fn outcomes(&self) -> &Outcomes {
        &self.quality.outcomes
    }

    /// Whether accepted outcomes may reorder equal-tier models.
    pub fn learned_routing(&self) -> bool {
        self.quality.learn
    }

    pub fn set_learned_routing(&mut self, on: bool) {
        self.quality.learn = on;
    }

    /// Images to send with the next turn (and only that one).
    pub fn attach_images(&mut self, images: Vec<PathBuf>) {
        self.images = images;
    }
}
