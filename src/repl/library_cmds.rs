//! `/sessions`: the library of saved sessions (search, marks, export, import, disk). The
//! rules live in [`crate::sessionlib`] and [`crate::sessionimport`]; this maps them onto
//! the screen.

use std::path::{Path, PathBuf};

use super::session_cmds::resolve;
use super::Repl;
use crate::handoff;
use crate::libcmd::SessionsCmd;
use crate::sessionimport::{self, Found};
use crate::sessionlib::{self, human_bytes, Mark, Query};
use crate::sessionlog::utc_stamp;
use crate::views::human_ago;

/// Most results printed at once; the rest are reachable by narrowing the search.
const SHOWN: usize = 15;

/// What `/sessions` remembers between commands: the files of the last listing, so that
/// `/sessions pin 2` means the second one shown.
#[derive(Debug, Default)]
pub(super) struct LibraryState {
    listing: Vec<PathBuf>,
}

impl Repl {
    pub(super) fn sessions(&mut self, cmd: SessionsCmd) {
        let style = self.out_style();
        let Some(state) = self.engine.state_dir().map(Path::to_path_buf) else {
            return self.console.out(&format!("  {}", style.dim("Saving sessions is switched off, so there is no library.")));
        };
        let moved = sessionlib::migrate_legacy(&state);
        if moved > 0 {
            self.console.out(&format!("  {}", style.dim(&format!("Moved {moved} session(s) saved by an older version into their project folders."))));
        }
        match cmd {
            SessionsCmd::Find(query) => self.find_sessions(&state, &query),
            SessionsCmd::Mark { n, mark } => self.mark_session(&state, n, &mark),
            SessionsCmd::Export { n, file } => self.export_session(&state, n, file.as_deref()),
            SessionsCmd::ImportFile(file) => self.import_file(&state, &file),
            SessionsCmd::ImportHistory { source, pick } => self.import_history(&state, source, pick),
            SessionsCmd::Disk => self.disk(&state),
            SessionsCmd::Clean { days } => {
                let (n, bytes) = sessionlib::clean(&state, days, self.now());
                let text = format!("removed {n} session file(s) older than {days} days or unreadable, freeing {}; pinned sessions are never removed", human_bytes(bytes));
                self.console.out(&format!("{} {text}", style.green("✓")));
            }
        }
    }

    fn find_sessions(&mut self, state: &Path, query: &Query) {
        let style = self.out_style();
        let now = self.now();
        let hits = sessionlib::search(sessionlib::scan_all(state), query, now);
        self.library.listing = hits.iter().map(|h| h.entry.path.clone()).collect();
        if hits.is_empty() {
            return self.console.out(&format!("  {}", style.dim("No saved session matches. Filters: provider: tag: file: since: project: pinned archived all.")));
        }
        let shown = &hits[..hits.len().min(SHOWN)];
        let mut lines = vec![style.bold(&format!("Sessions ({})", hits.len()))];
        lines.extend(sessionlib::render_hits(shown, now, &handoff::project_key(self.engine.dir()), style));
        if hits.len() > SHOWN {
            lines.push(style.dim(&format!("  ... and {} more: narrow with words or filters", hits.len() - SHOWN)));
        }
        lines.push(style.dim("  /sessions pin|archive|tag <n> ... · /sessions export <n> · /resume continues one of this folder's"));
        self.page_out(&lines.join("\n"));
    }

    /// The path of session `n` of the last listing.
    fn listed(&self, n: usize) -> Result<PathBuf, String> {
        self.library.listing.get(n - 1).cloned().ok_or_else(|| {
            if self.library.listing.is_empty() {
                "List sessions first with /sessions; the numbers refer to that list.".to_string()
            } else {
                format!("There is no session {n}; the last list had {}.", self.library.listing.len())
            }
        })
    }

    fn mark_session(&mut self, state: &Path, n: usize, mark: &Mark) {
        let style = self.out_style();
        let result = self.listed(n).and_then(|path| sessionlib::mark(state, &path, mark).map_err(|e| format!("could not save the mark: {e}")));
        let done = match mark {
            Mark::Pin(true) => format!("pinned session {n}: it stays at the top and is never deleted"),
            Mark::Pin(false) => format!("unpinned session {n}"),
            Mark::Archive(true) => format!("archived session {n}: /resume no longer lists it (find it with /sessions archived)"),
            Mark::Archive(false) => format!("session {n} is back in /resume"),
            Mark::Tag(t) => format!("tagged session {n} #{t}"),
            Mark::Untag(t) => format!("removed #{t} from session {n}"),
        };
        match result {
            Ok(()) => self.console.out(&format!("{} {done}", style.green("✓"))),
            Err(e) => self.console.out(&format!("{} {e}", style.yellow("⚠"))),
        }
    }

    fn export_session(&mut self, state: &Path, n: usize, file: Option<&str>) {
        let style = self.out_style();
        let path = match self.listed(n) {
            Ok(p) => p,
            Err(e) => return self.console.out(&format!("{} {e}", style.yellow("⚠"))),
        };
        let Some(entry) = sessionlib::scan_all(state).into_iter().find(|e| e.path == path) else {
            return self.console.out(&format!("{} that session is gone from disk", style.yellow("⚠")));
        };
        let dest = match file {
            Some(f) => resolve(f, self.engine.dir(), self.home.as_deref()),
            None => self.engine.dir().join(format!("jev-session-{}.json", utc_stamp(self.now()).1)),
        };
        match sessionlib::export_to(&entry, &dest) {
            Ok(()) => self.console.out(&format!("{} exported \u{201c}{}\u{201d} to {} {}", style.green("✓"), entry.title(), dest.display(), style.dim("· /sessions import <file> reads it back"))),
            Err(e) => self.console.out(&format!("{} not exported: {e}", style.yellow("⚠"))),
        }
    }

    fn import_file(&mut self, state: &Path, file: &str) {
        let style = self.out_style();
        let source = resolve(file, self.engine.dir(), self.home.as_deref());
        match sessionlib::import_file(state, self.engine.dir(), &source, self.now()) {
            Ok(_) => self.console.out(&format!("{} imported into this folder: /resume lists it", style.green("✓"))),
            Err(e) => self.console.out(&format!("{} not imported: {e}", style.yellow("⚠"))),
        }
    }

    fn import_history(&mut self, state: &Path, source: sessionimport::Source, pick: Option<usize>) {
        let style = self.out_style();
        let Some(home) = self.home.as_deref().map(Path::new) else {
            return self.console.out(&format!("{} cannot find your home folder", style.yellow("⚠")));
        };
        let found: Vec<Found> = sessionimport::discover(home, self.engine.dir()).into_iter().filter(|f| f.source == source).collect();
        if found.is_empty() {
            return self.console.out(&format!("  {}", style.dim(&format!("No {} history for this folder.", source.label()))));
        }
        let now = self.now();
        let Some(n) = pick else {
            let mut lines = vec![style.bold(&format!("{} histories of this folder", source.label()))];
            for (i, f) in found.iter().enumerate() {
                let title = sessionimport::load(f).ok().and_then(|t| t.first().map(|t| t.prompt.split_whitespace().collect::<Vec<_>>().join(" "))).unwrap_or_default();
                let title: String = title.chars().take(70).collect();
                lines.push(format!("{:>3}. {title}", i + 1));
                lines.push(format!("     {}", style.dim(&human_ago(now.saturating_sub(f.modified_ms)))));
            }
            lines.push(style.dim("  Import one with /sessions import <claude|codex> <n>."));
            return self.page_out(&lines.join("\n"));
        };
        let Some(chosen) = found.get(n - 1) else {
            return self.console.out(&format!("  {}", style.dim(&format!("There is no history {n}; there are {}.", found.len()))));
        };
        let result = sessionimport::load(chosen)
            .map_err(|e| e.to_string())
            .and_then(|turns| {
                let count = turns.len();
                let saved = sessionimport::to_saved(turns, self.engine.dir(), chosen.modified_ms);
                sessionlib::adopt(state, self.engine.dir(), saved, now).map(|_| count).map_err(|e| e.to_string())
            });
        match result {
            Ok(count) => self.console.out(&format!("{} imported {count} turns from {}: /resume continues it on any provider", style.green("✓"), source.label())),
            Err(e) => self.console.out(&format!("{} not imported: {e}", style.yellow("⚠"))),
        }
    }

    fn disk(&mut self, state: &Path) {
        let style = self.out_style();
        let usage = sessionlib::disk_use(&sessionlib::scan_all(state));
        let mut lines = vec![
            style.bold("Saved sessions"),
            format!("  {} sessions, {} on disk ({} pinned, {} archived)", usage.sessions, human_bytes(usage.bytes), usage.pinned, usage.archived),
        ];
        for (project, n, bytes) in usage.by_project.iter().take(8) {
            lines.push(format!("  {:>9}  {n:>3}  {}", human_bytes(*bytes), sessionlib::project_name(project)));
        }
        lines.push(style.dim("  /sessions clean [days] removes older, unpinned sessions."));
        self.page_out(&lines.join("\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::availability::{Availability, State};
    use crate::backend::ProviderId;
    use crate::testkit::capture::Buf;
    use crate::console::{Console, ConsoleConfig};
    use crate::handoff::{Sessions, TurnRecord};
    use crate::providers::Runtime;
    use crate::router::Router;
    use crate::turn::Engine;
    use crate::typesafe::TypeSafe;
    use crate::uifmt::Style;

    struct Rig {
        repl: Repl,
        out: Buf,
        state: PathBuf,
        project: PathBuf,
    }

    fn rig(name: &str) -> Rig {
        let root = std::env::temp_dir().join(format!("jev-libcmd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (state, project) = (root.join("state"), root.join("project"));
        std::fs::create_dir_all(&project).unwrap();
        let avail = vec![Availability { provider: ProviderId::Claude, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true }];
        let engine = Engine::new(Runtime::for_tests(avail), Router::new(TypeSafe::with_url("k", "http://127.0.0.1:1"), 1.0), project.clone())
            .with_turn_log(None)
            .with_persistence(Some(state.clone()));
        let out = Buf::default();
        let cfg = ConsoleConfig { out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: project.clone(), stream_answers: true };
        let console = Console::new(Box::new(out.clone()), Box::new(Buf::default()), cfg);
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Rig { repl: Repl::new(engine, console, Some(home.to_string_lossy().into_owned()), false), out, state, project }
    }

    fn save(r: &Rig, prompt: &str, reply: &str) {
        let mut sessions = Sessions::new();
        sessions.record(TurnRecord { prompt: prompt.into(), provider: ProviderId::Codex, model: "m".into(), reply: reply.into(), tools: vec![], ok: true });
        let now = crate::log::now_ms();
        let saved = handoff::SavedSession { project: handoff::project_key(&r.project), saved_ms: now, sessions, current: None };
        let path = handoff::new_session_file(&r.state, &r.project, now + prompt.len() as u64);
        handoff::write_atomic(&path, &saved.render()).unwrap();
    }

    fn run(r: &mut Rig, line: &str) -> String {
        r.out.clear();
        match crate::commands::parse(line) {
            crate::commands::Command::Sessions(cmd) => r.repl.sessions(cmd),
            other => panic!("{line} parsed as {other:?}"),
        }
        r.out.text()
    }

    #[test]
    fn list_search_pin_tag_archive_and_resume_list_agree() {
        let mut r = rig("flow");
        save(&r, "fix the lexer", "done");
        save(&r, "write the docs", "written");
        let all = run(&mut r, "/sessions");
        assert!(all.contains("Sessions (2)") && all.contains("fix the lexer") && all.contains("write the docs"), "{all}");
        let found = run(&mut r, "/sessions docs");
        assert!(found.contains("Sessions (1)") && found.contains("write the docs") && !found.contains("lexer"), "{found}");
        assert!(run(&mut r, "/sessions pin 1").contains("pinned session 1"));
        assert!(run(&mut r, "/sessions tag 1 docs").contains("tagged session 1 #docs"));
        let tagged = run(&mut r, "/sessions tag:docs");
        assert!(tagged.contains("★") && tagged.contains("#docs"), "{tagged}");
        run(&mut r, "/sessions");
        run(&mut r, "/sessions archive 1");
        assert_eq!(r.repl.engine.resumable().unwrap().len(), 1, "archived leaves /resume");
        assert!(run(&mut r, "/sessions archived").contains("archived"));
        assert!(run(&mut r, "/sessions pin 9").contains("There is no session 9"));
    }

    #[test]
    fn numbers_need_a_listing_first() {
        let mut r = rig("nolist");
        assert!(run(&mut r, "/sessions pin 1").contains("List sessions first"));
        assert!(run(&mut r, "/sessions xyzzy").contains("No saved session matches"));
    }

    #[test]
    fn export_then_import_and_disk_and_clean() {
        let mut r = rig("portable");
        save(&r, "port me", "ok");
        run(&mut r, "/sessions");
        let exported = run(&mut r, "/sessions export 1 out.json");
        assert!(exported.contains("exported") && r.project.join("out.json").is_file(), "{exported}");
        assert!(run(&mut r, "/sessions export 1 out.json").contains("not exported"));
        assert!(run(&mut r, "/sessions import out.json").contains("imported"));
        assert_eq!(r.repl.engine.resumable().unwrap().len(), 2);
        assert!(run(&mut r, "/sessions disk").contains("2 sessions"));
        assert!(run(&mut r, "/sessions clean 365").contains("removed 0"));
    }

    #[test]
    fn history_import_lists_then_imports_from_claude_code() {
        let mut r = rig("history");
        let home = PathBuf::from(r.repl.home.clone().unwrap());
        assert!(run(&mut r, "/sessions import claude").contains("No Claude Code history"));
        let dir = sessionimport::claude_project_dir(&home, &r.project);
        std::fs::create_dir_all(&dir).unwrap();
        let line = |t: &str, body: &str| format!("{{\"type\":\"{t}\",\"message\":{body}}}\n");
        let text = line("user", "{\"role\":\"user\",\"content\":\"explain the build\"}") + &line("assistant", "{\"content\":[{\"type\":\"text\",\"text\":\"It uses cargo.\"}]}");
        std::fs::write(dir.join("one.jsonl"), text).unwrap();
        let listed = run(&mut r, "/sessions import claude");
        assert!(listed.contains("explain the build"), "{listed}");
        assert!(run(&mut r, "/sessions import claude 1").contains("imported 1 turns"));
        let resumable = r.repl.engine.resumable().unwrap();
        assert_eq!(resumable[0].title, "explain the build");
        assert!(run(&mut r, "/sessions import claude 5").contains("There is no history 5"));
    }
}
