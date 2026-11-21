//! The session side of [`crate::approval::application::extensions`]: `/extensions`, `/recipe`, `/hooks`, and the custom
//! commands, skills and recipes typed as `/<name> [arguments]`.

use std::sync::Arc;

use crate::tui::repl::session::{Flow, Io, Repl};
use crate::approval::application::extension_hooks as hooks;
use crate::approval::application::extension_hooks::Hooks;
use crate::approval::application::extension_library as library;
use crate::approval::application::extension_library::{Invocation, Kind, Library};
use crate::approval::application::extension_market as market;
use crate::tui::render::extension_view as view;
use crate::approval::application::extensions::{ExtCmd, Places};
use crate::tui::app::layered_settings as layers;
use crate::approval::domain::profile::Profile;
use crate::turn::domain::turn_status::TurnStatus;

/// The line for a skill name that is not one: the closest real name when there is one.
fn no_skill(lib: &Library, name: &str, style: crate::tui::render::Style) -> String {
    let near = lib
        .of_kind(Kind::Skill)
        .map(|e| (crate::tui::command::edit_distance(name, &e.name), e.name.as_str()))
        .filter(|(d, n)| *d <= 2 || n.starts_with(name))
        .min_by_key(|(d, _)| *d)
        .map(|(_, n)| n);
    let hint = match near {
        Some(n) => format!(" Did you mean {}?", style.cyan(&format!("/skill {n}"))),
        None => format!(" {}", style.dim("/skill lists them")),
    };
    format!("{} no skill named {name}.{hint}", style.yellow("⚠"))
}

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
                let installed = places.user.as_deref().map(market::listing::list).unwrap_or_default();
                let text = format!("{}\n{}\n{}", view::library_text(&lib, style), style.bold("extensions"), view::installed_text(&installed, style));
                self.page_out(&text);
            }
            ExtCmd::Recipes => self.page_out(&view::recipes_text(&Library::discover(&places), style)),
            ExtCmd::RunRecipe { name, args } => {
                let lib = Library::discover(&places);
                match lib.find(&name).filter(|e| e.kind == crate::approval::application::extension_library::Kind::Recipe) {
                    Some(entry) => return self.run_invocation(entry.invoke(&args), io).await,
                    None => self.console.out(&format!("{} no recipe named {name}. {}", style.yellow("⚠"), style.dim("/recipe lists them"))),
                }
            }
            ExtCmd::Skills => self.page_out(&view::skills_text(&Library::discover(&places), style)),
            ExtCmd::RunSkill { name, args } => {
                let lib = Library::discover(&places);
                match lib.find(&name).filter(|e| e.kind == Kind::Skill) {
                    Some(entry) => return self.run_invocation(entry.invoke(&args), io).await,
                    None => self.console.out(&no_skill(&lib, &name, style)),
                }
            }
            ExtCmd::ShowSkill(name) => {
                let lib = Library::discover(&places);
                match lib.find(&name).filter(|e| e.kind == Kind::Skill) {
                    Some(entry) => self.page_out(&view::skill_text(entry, style)),
                    None => self.console.out(&no_skill(&lib, &name, style)),
                }
            }
            ExtCmd::NewSkill { name, description } => {
                let taken = Library::discover(&places).find(&name).map(|e| format!("/{name} is already a {} ({})", e.kind.noun(), e.origin));
                match taken.map_or_else(|| library::create_skill(&places, &name, &description), Err) {
                    Ok(path) => {
                        self.console.out(&format!("{} started skill {}", style.green("✓"), style.cyan(&format!("/{name}"))));
                        self.console.out(&style.dim(&format!("  edit {} to say what it should do, then run /skill {name}", path.display())));
                    }
                    Err(e) => self.console.out(&format!("{} {e}", style.yellow("⚠"))),
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
        match hooks::trust::trust_project_full(user, &places.project) {
            Ok((newly, approved, settings_lines)) => {
                let loaded = Hooks::load(places);
                self.page_out(&view::approved_hooks_text(&approved, style));
                let settings = view::approved_settings_text(&settings_lines, style);
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
