//! Commands on the session itself: `/rename` and `/resume`.

use crate::session::domain::saved_session::ResumeError;
use crate::tui::render as uifmt;
use crate::tui::repl::session::Repl;
use crate::tui::views::session_picker as picker;
use crate::tui::views::status_facts as views;
use crate::tui::tty::terminal_guard as terminal;

impl Repl {
    /// `/rename [name]`: name the session, or say what it is called.
    pub(super) fn rename(&mut self, name: Option<&str>) {
        let style = self.out_style();
        let Some(name) = name else {
            let text = match self.engine.sessions.name() {
                Some(n) => format!("This session is called \u{201c}{n}\u{201d}. Rename it with /rename <name>."),
                None => "This session has no name yet. Name it with /rename <name>; /resume then lists it by that name.".to_string(),
            };
            return self.console.out(&format!("  {}", style.dim(&text)));
        };
        self.engine.sessions.set_name(Some(name));
        let Some(set) = self.engine.sessions.name().map(String::from) else {
            return self.console.out(&format!("  {}", style.dim("A name needs at least one word.")));
        };
        let saved = self.engine.sessions.turns() > 0;
        let saving = self.engine.save_now();
        self.said_if_failed(saving, "the name");
        let when = if saved { "" } else { " (kept with your first message)" };
        self.console.out(&format!("{} this session is now called \u{201c}{set}\u{201d}{}", style.green("✓"), style.dim(when)));
    }


    /// `/resume [number | words]`: continue one of this folder's saved sessions. With
    /// several to choose from, a terminal gets the list to pick from; any other output gets
    /// it numbered, and `/resume 2` takes one.
    pub(super) fn resume(&mut self, query: Option<&str>) {
        let style = self.out_style();
        let all = match self.engine.resumable() {
            Ok(all) => all,
            Err(e) => return self.resume_failed(&e),
        };
        if all.is_empty() {
            return self.resume_failed(&ResumeError::NotFound);
        }
        let now = self.now();
        let candidates = match query.map(|q| (q, q.parse::<usize>())) {
            Some((_, Ok(n))) => match n.checked_sub(1).and_then(|i| all.get(i)) {
                Some(one) => vec![one.clone()],
                None => {
                    let text = format!("There is no session {n}; there are {}.", all.len());
                    return self.console.out(&format!("  {}", style.dim(&text)));
                }
            },
            Some((words, Err(_))) => {
                let found: Vec<_> = all.iter().filter(|e| picker::matches(e, words)).cloned().collect();
                if found.is_empty() {
                    return self.console.out(&format!("  {}", style.dim(&format!("No saved session matches \"{words}\"."))));
                }
                found
            }
            None => all,
        };
        let chosen = if candidates.len() == 1 {
            Some(0)
        } else if self.paging && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            self.console.with_terminal(|| picker::run(candidates.clone(), now, style, terminal::rows(), terminal::width()))
        } else {
            let mut lines = picker::numbered(&candidates, now, terminal::width(), style);
            lines.push(style.dim("  Resume one with /resume <number>, or narrow the list with /resume <words>."));
            self.console.out(&lines.join("\n"));
            return;
        };
        let Some(entry) = chosen.and_then(|i| candidates.get(i)) else {
            return self.console.out(&format!("  {}", style.dim("Resume cancelled.")));
        };
        match self.engine.resume_from(&entry.path) {
            Ok(info) => {
                self.last_summary = None;
                self.log.clear();
                self.carry.clear();
                self.console.out(&views::startup::resumed_line(&info, self.now(), style));
            }
            Err(e) => self.resume_failed(&e),
        }
    }

    pub(crate) fn resume_failed(&self, e: &ResumeError) {
        let style = self.out_style();
        match e {
            ResumeError::NotFound | ResumeError::Disabled => {
                self.console.out(&format!("  {}", style.dim(&format!("Nothing to resume: {e}"))));
            }
            _ => {
                let steps = ["Start fresh with `/new`".to_string()];
                self.console.out(&uifmt::problem_block("The saved session can't be read", &[e.to_string()], &steps, style));
            }
        }
    }
}
