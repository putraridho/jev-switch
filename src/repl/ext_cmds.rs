//! The session side of [`crate::ext`]: `/extensions`, `/recipe`, `/hooks`, and the custom
//! commands, skills and recipes typed as `/<name> [arguments]`.

use std::sync::Arc;

use super::{Flow, Io, Repl};
use crate::ext::hooks::{self, Hooks};
use crate::ext::library::{Invocation, Library};
use crate::ext::{market, view, ExtCmd, Places};
use crate::layers;
use crate::profile::Profile;
use crate::turn::TurnStatus;

impl Repl {
    fn ext_places(&self) -> Places {
        Places::from_env(self.engine.dir())
    }

    pub(super) async fn ext_command(&mut self, cmd: ExtCmd, io: &mut Io<'_>) -> Flow {
        let style = self.out_style();
        let places = self.ext_places();
        match cmd {
            ExtCmd::List => {
                let lib = Library::discover(&places);
                let installed = places.user.as_deref().map(market::list).unwrap_or_default();
                let text = format!("{}\n{}\n{}", view::library_text(&lib, style), style.bold("extensions"), view::installed_text(&installed, style));
                self.page_out(&text);
            }
            ExtCmd::Recipes => self.page_out(&view::recipes_text(&Library::discover(&places), style)),
            ExtCmd::RunRecipe { name, args } => {
                let lib = Library::discover(&places);
                match lib.find(&name).filter(|e| e.kind == crate::ext::library::Kind::Recipe) {
                    Some(entry) => return self.run_invocation(entry.invoke(&args), io).await,
                    None => self.console.out(&format!("{} no recipe named {name}. {}", style.yellow("⚠"), style.dim("/recipe lists them"))),
                }
            }
            ExtCmd::Settings => {
                let profile = std::env::var("JEV_PROFILE").ok().filter(|p| !p.is_empty());
                let available = places.user.as_deref().map(layers::profiles).unwrap_or_default();
                let text = layers::settings_text(&layers::Layered::load(&places, profile.as_deref()), profile.as_deref(), &available, style);
                self.page_out(&text);
            }
            ExtCmd::Hooks => self.page_out(&view::hooks_text(&Hooks::load(&places), style)),
            ExtCmd::TrustHooks => self.trust_hooks(&places),
        }
        Flow::Continue
    }

    fn trust_hooks(&mut self, places: &Places) {
        let style = self.out_style();
        let Some(user) = &places.user else {
            self.console.out(&format!("{} no config folder to remember the trust in (HOME is not set)", style.yellow("⚠")));
            return;
        };
        match hooks::trust_project_with_preview(user, &places.project) {
            Ok((newly, approved)) => {
                let loaded = Hooks::load(places);
                self.page_out(&view::approved_hooks_text(&approved, style));
                let settings = view::approved_settings_text(&hooks::claude_settings_preview(&places.project), style);
                if !settings.is_empty() {
                    self.page_out(&settings);
                }
                if loaded.skipped_untrusted > 0 || !loaded.problems.is_empty() {
                    self.page_out(&view::hooks_text(&loaded, style));
                }
                self.engine.set_hooks(Arc::new(loaded));
                let said = if newly { "saved trust for this project's hooks" } else { "this project's current hooks were already trusted" };
                self.console.out(&format!("{} {said}", style.green("✓")));
            }
            Err(e) => self.console.out(&format!("{} could not save the trust: {e}", style.yellow("⚠"))),
        }
    }

    /// `/<name> [arguments]` for a custom command, skill or recipe. `None`: no such entry.
    pub(super) async fn custom(&mut self, typed: &str, args: &str, io: &mut Io<'_>) -> Option<Flow> {
        let lib = Library::discover(&self.ext_places());
        let entry = lib.find(typed.trim_start_matches('/'))?;
        Some(self.run_invocation(entry.invoke(args), io).await)
    }

    /// Run the prompts of an invocation as turns, one after the other, under its mode (then
    /// put back). Stops at the first turn that did not answer, so a failed step does not
    /// send the next one on top of nothing.
    async fn run_invocation(&mut self, inv: Invocation, io: &mut Io<'_>) -> Flow {
        let style = self.out_style();
        let before = self.engine.mode();
        if let Some(asked) = inv.mode {
            // A command, skill or recipe may narrow the mode (plan for a review) but never widen
            // it: a file committed to a repo must not switch a session to Auto, or past the
            // repo's own limit.
            let allowed = self.within_team_limit(asked);
            if Profile::of_mode(allowed) > Profile::of_mode(before) {
                self.warn(&format!("{} asks for {allowed} mode; staying in {before} mode (use Shift-Tab or /mode to widen it)", inv.label));
            } else {
                self.engine.set_mode(allowed);
            }
        }
        let total = inv.prompts.len();
        let mut flow = Flow::Continue;
        for (i, (title, prompt)) in inv.prompts.iter().enumerate() {
            let label = if total > 1 {
                self.console.out(&style.dim(&format!("  step {}/{total}: {title}", i + 1)));
                format!("{} [{}/{total}]", inv.label, i + 1)
            } else {
                inv.label.clone()
            };
            let sent = self.with_carry(prompt);
            flow = self.run_recorded(&sent, &label, io).await;
            if flow != Flow::Continue {
                break;
            }
            if self.log.turns().last().map(|t| t.status) != Some(TurnStatus::Answered) {
                if i + 1 < total {
                    self.console.out(&format!("{} stopped at step {}/{total}: it did not finish", style.yellow("⚠"), i + 1));
                }
                break;
            }
        }
        self.engine.set_mode(before);
        flow
    }
}
