//! `Repl::handle`: the dispatch from a parsed command to what does it.

use crate::command::Command;
use crate::render as uifmt;
use crate::repl::session::{Flow, Io, Repl};
use crate::repl::session_cmds;
use crate::views::status_facts as views;
use jev_routing::domain::decide::Pin as ModelPin;

impl Repl {
    /// Run one line's worth of input.
    pub async fn handle(&mut self, cmd: Command, io: &mut Io<'_>) -> Flow {
        let style = self.out_style();
        match cmd {
            Command::Empty => {}
            Command::Quit => return Flow::Quit,
            Command::Prompt(text) => return self.prompt(&text, io).await,
            Command::Help => self.page_out(&uifmt::help_text(style, self.console.width())),
            Command::Learn(learn) => self.page_out(&learn.render(style, self.console.width())),
            Command::Status => {
                let facts = views::providers::status_facts(&self.engine, self.last_summary.as_ref(), self.home.as_deref(), self.now());
                self.page_out(&uifmt::status_text(&facts, style));
            }
            Command::Providers { refresh } => self.providers(refresh).await,
            Command::ProbeProviders => self.probe_providers().await,
            Command::Mcp(cmd) => self.mcp(cmd),
            Command::Doctor => {
                self.console.out(&style.dim("  checking the setup..."));
                let text = self.doctor_text().await;
                self.page_out(&text);
            }
            Command::ShowModel => self.console.out(&format!("  model: {}", self.engine.pin())),
            Command::SetPin(pin) => self.set_pin(pin),
            Command::SetRouting(on) => {
                self.engine.set_enabled(on);
                self.remember_flag(crate::app::preferences::Flag::Routing, on, "routing");
                let text = if on { "on".to_string() } else { format!("off {}", style.dim("(Claude only)")) };
                self.console.out(&format!("  routing: {text}"));
            }
            Command::SetOrchestrate(on) => {
                self.engine.set_orchestrate(on);
                self.remember_flag(crate::app::preferences::Flag::Orchestrate, on, "orchestrate");
                self.console.out(&format!("  orchestrate: {}", on_off(on)));
            }
            Command::ShowOrchestrate => self.console.out(&format!("  orchestrate: {}", on_off(self.engine.orchestrate()))),
            Command::Verify(None) => self.console.out(&format!("  verify: {}", self.engine.verify_mode().label())),
            Command::Verify(Some(mode)) => {
                self.engine.set_verify_mode(mode);
                self.console.out(&format!("  verify: {}", self.engine.verify_mode().label()));
            }
            Command::SetThinking(on) => {
                self.console.set_show_thinking(on);
                self.remember_flag(crate::app::preferences::Flag::Thinking, on, "thinking");
                self.console.out(&format!("  thinking: {}", on_off(on)));
            }
            Command::ShowThinking => self.console.out(&format!("  thinking: {}", on_off(self.console.show_thinking()))),
            Command::Trust(cmd) => return self.trust(cmd, io).await,
            Command::SetMode(mode) => {
                let mode = self.adopt_mode(mode);
                self.console.out(&format!("  mode: {}", views::prompt::mode_line(mode)));
            }
            Command::ShowMode => self.console.out(&format!("  mode: {}", views::prompt::mode_line(self.engine.mode()))),
            Command::New => {
                self.fresh_session();
                self.console.out(&format!("{} new session", style.green("✓")));
            }
            Command::Clear => {
                self.console.clear_screen();
                self.fresh_session();
                self.console.out(&format!("{} new session", style.green("✓")));
            }
            Command::Compact(focus) => return self.compact(focus.as_deref(), io).await,
            Command::Context => self.page_out(&self.context_text()),
            Command::Cost => self.page_out(&self.cost_text()),
            Command::Cache => self.page_out(&self.cache_text()),
            Command::Effort(setting) => self.effort(setting),
            Command::Guard(cmd) => {
                let plans: Vec<(String, f64)> = jev_shared::ALL_PROVIDERS
                    .into_iter()
                    .filter_map(|p| self.engine.runtime.plan_utilization(p).map(|u| (p.to_string(), u)))
                    .collect();
                let (guards, settings) = self.engine.guards_and_settings();
                let text = crate::repl::guard_commands::run(cmd, guards, settings, &plans);
                self.page_out(&text);
            }
            Command::Why => {
                let pin = self.engine.pin().to_string();
                self.page_out(&uifmt::why_text(self.console.last_route().as_ref(), &pin, self.engine.enabled(), style));
            }
            Command::Export(path) => self.export(path.as_deref()),
            Command::Copy => self.copy(),
            Command::Edit => return self.edit(io).await,
            Command::Init => return self.canned(session_cmds::INIT_PROMPT, "/init", None, io).await,
            Command::Memory => self.memory(),
            Command::Review(focus) => {
                let prompt = session_cmds::review_prompt(focus.as_deref());
                return self.canned(&prompt, "/review", Some(jev_approval::domain::mode::Mode::Plan), io).await;
            }
            Command::Config => self.page_out(&self.config_text()),
            Command::Agents => self.page_out(&crate::views::agent_stored::agents_text(&self.stored_workers(), self.now(), self.console.width(), style)),
            Command::Orchestration => {
                let text = crate::views::agent_stored::orchestration_text(self.engine.orchestrate(), &self.stored_workers(), self.now(), self.console.width(), style);
                self.page_out(&text);
            }
            Command::TerminalSetup { install } => self.terminal_setup(install, io),
            Command::Update => self.update(io),
            Command::Shell(command) => self.shell(&command),
            Command::Note(note) => self.note(&note),
            Command::Resume(query) => self.resume(query.as_deref()),
            Command::Rename(name) => self.rename(name.as_deref()),
            Command::Workflows => {
                let text = self.workflows_text();
                self.page_out(&text);
            }
            Command::Sessions(cmd) => self.sessions(cmd),
            Command::Project(cmd) => return self.project(cmd, io).await,
            Command::Permissions(cmd) => self.permissions(cmd),
            Command::Diff(args) => self.diff(args.as_deref(), io),
            Command::Rewind(turn) => self.rewind(turn, io),
            Command::Undo => self.undo(io).await,
            Command::Invalid(message) => self.console.out(&format!("{} {message}", style.yellow("⚠"))),
            Command::Ext(cmd) => return self.ext_command(cmd, io).await,
            Command::Unknown { typed, args, suggestion } => {
                if let Some(flow) = self.custom(&typed, &args, io).await {
                    return flow;
                }
                let mut text = format!("{} unknown command {typed}", style.red("✗"));
                if let Some(s) = suggestion {
                    text.push_str(&format!(". Did you mean {}?", style.cyan(s)));
                }
                text.push_str(&format!(" Type {} for the commands.", style.cyan("/help")));
                self.console.out(&text);
            }
        }
        Flow::Continue
    }

    fn set_pin(&mut self, pin: ModelPin) {
        self.console.out(&format!("  model: {pin}"));
        self.engine.set_pin(pin);
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}
