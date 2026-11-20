//! The session library: every saved session of every project, searched, listed with its
//! marks (tags, pinned, archived), exported to a file and imported back, measured and
//! cleaned. Pure over a state folder, so each rule is a unit test on a temp folder; the
//! `/sessions` command in the REPL only maps text onto these functions.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::backend::ProviderId;
use crate::session::infrastructure::session_file;
use crate::shared::infrastructure::private_file::write_atomic;
use crate::session::domain::saved_session::SavedSession;
use crate::session::domain::session::Sessions;
use crate::session::infrastructure::session_meta::{Meta, MetaStore};
use crate::tui::render::Style;
use crate::tui::views::status_facts::human_ago;

const DAY_MS: u64 = 86_400_000;

/// One saved session found on disk, with what is said about it.
#[derive(Debug, Clone)]
pub struct LibraryEntry {
    pub path: PathBuf,
    pub saved: SavedSession,
    pub bytes: u64,
    pub meta: Meta,
}

impl LibraryEntry {
    pub fn title(&self) -> String {
        self.saved.sessions.title().unwrap_or_else(|| "(no message)".to_string())
    }

    /// Everything searchable about the session, lower-cased, one line per piece.
    fn haystack(&self) -> Vec<String> {
        let mut lines = vec![self.title().to_lowercase()];
        for turn in self.saved.sessions.log() {
            lines.extend(turn.prompt.lines().map(str::to_lowercase));
            lines.extend(turn.reply.lines().map(str::to_lowercase));
            lines.extend(turn.tools.iter().map(|t| t.to_lowercase()));
            lines.push(format!("{} {}", turn.provider, turn.model).to_lowercase());
        }
        lines
    }

    fn mentions_file(&self, needle: &str) -> bool {
        self.saved.sessions.log().iter().any(|t| t.tools.iter().any(|tool| tool.to_lowercase().contains(needle)))
    }

    fn used(&self, provider: ProviderId) -> bool {
        self.saved.sessions.log().iter().any(|t| t.provider == provider)
    }
}

/// Which sessions a listing is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// Not archived.
    #[default]
    Active,
    Archived,
    All,
}

/// What a person asked the library for: free words plus `key:value` filters, e.g.
/// `parser provider:codex tag:bug file:lexer.rs since:7d project:jev`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub words: Vec<String>,
    pub provider: Option<ProviderId>,
    pub tag: Option<String>,
    pub file: Option<String>,
    pub project: Option<String>,
    /// Only sessions saved within this many milliseconds.
    pub since_ms: Option<u64>,
    pub pinned_only: bool,
    pub scope: Scope,
}

impl Query {
    /// `Err` says which filter was not understood.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut q = Self::default();
        for token in text.split_whitespace() {
            match token.split_once(':') {
                Some(("provider", v)) => q.provider = Some(ProviderId::from_str(v).map_err(|_| format!("unknown provider \"{v}\""))?),
                Some(("tag", v)) => q.tag = Some(crate::session::infrastructure::session_meta::clean_tag(v).ok_or_else(|| format!("\"{v}\" is not a tag"))?),
                Some(("file", v)) if !v.is_empty() => q.file = Some(v.to_lowercase()),
                Some(("project", v)) if !v.is_empty() => q.project = Some(v.to_lowercase()),
                Some(("since", v)) => q.since_ms = Some(parse_days(v).ok_or_else(|| format!("since:{v} should look like since:7d or since:2w"))?),
                _ if token == "pinned" => q.pinned_only = true,
                _ if token == "archived" => q.scope = Scope::Archived,
                _ if token == "all" => q.scope = Scope::All,
                _ => q.words.push(token.to_lowercase()),
            }
        }
        Ok(q)
    }

    fn admits(&self, e: &LibraryEntry, now_ms: u64) -> bool {
        let in_scope = match self.scope {
            Scope::Active => !e.meta.archived,
            Scope::Archived => e.meta.archived,
            Scope::All => true,
        };
        in_scope
            && (!self.pinned_only || e.meta.pinned)
            && self.provider.is_none_or(|p| e.used(p))
            && self.tag.as_ref().is_none_or(|t| e.meta.tags.contains(t))
            && self.file.as_ref().is_none_or(|f| e.mentions_file(f))
            && self.project.as_ref().is_none_or(|p| e.saved.project.to_lowercase().contains(p))
            && self.since_ms.is_none_or(|ms| now_ms.saturating_sub(e.saved.saved_ms) <= ms)
    }
}

/// `7d` or `2w` in milliseconds.
fn parse_days(text: &str) -> Option<u64> {
    // Split before the last character, not the last byte: `7日` must be refused, not panic.
    let (at, unit) = text.char_indices().last()?;
    let n: u64 = text[..at].parse().ok()?;
    match unit {
        'd' => Some(n.saturating_mul(DAY_MS)),
        'w' => Some(n.saturating_mul(7 * DAY_MS)),
        _ => None,
    }
}

/// One session that answers a query, and the line that matched its words.
#[derive(Debug, Clone)]
pub struct Hit {
    pub entry: LibraryEntry,
    pub snippet: Option<String>,
}

fn session_dirs(state_dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(state_dir.join("sessions")).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())).collect()
}

/// Every readable session file under `<state>/sessions/`, in any project, newest first.
/// Damaged files are skipped (they stay on disk; `clean` removes them).
pub fn scan_all(state_dir: &Path) -> Vec<LibraryEntry> {
    let meta = MetaStore::load(state_dir);
    let files = session_dirs(state_dir).into_iter().flat_map(|p| {
        if p.is_dir() {
            std::fs::read_dir(p).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())).collect()
        } else {
            vec![p]
        }
    });
    let mut found: Vec<LibraryEntry> = files
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let saved = SavedSession::parse(&text).ok().filter(|s| s.sessions.turns() > 0)?;
            Some(LibraryEntry { bytes: text.len() as u64, meta: meta.get(&path), path, saved })
        })
        .collect();
    found.sort_by(|a, b| (b.saved.saved_ms, &b.path).cmp(&(a.saved.saved_ms, &a.path)));
    found
}

/// The sessions that answer `query`: pinned first, then newest first. With words, only
/// sessions whose text holds every word, each with the first line that held one.
pub fn search(entries: Vec<LibraryEntry>, query: &Query, now_ms: u64) -> Vec<Hit> {
    let mut hits: Vec<Hit> = entries
        .into_iter()
        .filter(|e| query.admits(e, now_ms))
        .filter_map(|entry| {
            if query.words.is_empty() {
                return Some(Hit { entry, snippet: None });
            }
            let lines = entry.haystack();
            let everywhere = query.words.iter().all(|w| lines.iter().any(|l| l.contains(w.as_str())));
            let snippet = everywhere.then(|| snippet_for(&entry, &query.words[0])).flatten();
            everywhere.then_some(Hit { entry, snippet })
        })
        .collect();
    hits.sort_by_key(|h| !h.entry.meta.pinned);
    hits
}

/// The first original line holding `word`, shortened around it.
fn snippet_for(e: &LibraryEntry, word: &str) -> Option<String> {
    let log = e.saved.sessions.log();
    let lines = log.iter().flat_map(|t| t.prompt.lines().chain(t.reply.lines()).chain(t.tools.iter().map(String::as_str)));
    let line = lines.into_iter().find(|l| l.to_lowercase().contains(word))?.trim().to_string();
    // Lowercasing can turn one character into several (U+0130 becomes `i` and a combining dot),
    // so neither byte nor character offsets carry over. The word is found in the whole-string
    // lowering (the one the match used: a word-final Σ becomes ς there), and mapped back with a
    // table built char by char of where each lowered byte's character starts in `line`. The only
    // mapping that depends on context is σ/ς, both two bytes, so the two lowerings line up.
    let lowered = line.to_lowercase();
    let mut origin = Vec::with_capacity(lowered.len());
    for (i, c) in line.char_indices() {
        let width: usize = c.to_lowercase().map(char::len_utf8).sum();
        origin.extend(std::iter::repeat_n(i, width));
    }
    let at = lowered.find(word).and_then(|found| origin.get(found).copied()).unwrap_or(0);
    let start = line[..at].char_indices().rev().nth(30).map_or(0, |(i, _)| i);
    let chars: String = line[start..].chars().take(90).collect();
    Some(if start > 0 { format!("...{chars}") } else { chars })
}

/// A hit list as printed: a number, the title, then marks, age and the matched line.
pub fn render_hits(hits: &[Hit], now_ms: u64, current_project: &str, style: Style) -> Vec<String> {
    hits.iter()
        .enumerate()
        .flat_map(|(n, h)| {
            let e = &h.entry;
            let pin = if e.meta.pinned { style.yellow("★ ") } else { String::new() };
            let title: String = e.title().chars().take(70).collect();
            let turns = if e.saved.sessions.turns() == 1 { "1 turn".to_string() } else { format!("{} turns", e.saved.sessions.turns()) };
            let mut facts = vec![human_ago(now_ms.saturating_sub(e.saved.saved_ms)), turns];
            let providers: Vec<&str> = e.saved.sessions.providers_used().iter().map(ProviderId::as_str).collect();
            facts.push(providers.join("+"));
            if e.saved.project != current_project {
                facts.push(project_name(&e.saved.project));
            }
            facts.extend(e.meta.tags.iter().map(|t| format!("#{t}")));
            if e.meta.archived {
                facts.push("archived".to_string());
            }
            let mut rows = vec![format!("{:>3}. {pin}{title}", n + 1), format!("     {}", style.dim(&facts.join(" · ")))];
            if let Some(s) = &h.snippet {
                rows.push(format!("     {}", style.dim(&format!("\u{201c}{s}\u{201d}"))));
            }
            rows
        })
        .collect()
}

/// The last folder of a project path.
pub fn project_name(project: &str) -> String {
    Path::new(project).file_name().map_or_else(|| project.to_string(), |n| n.to_string_lossy().into_owned())
}

// ------------------------------------------------------------------ marks

/// A change to one session's marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    Pin(bool),
    Archive(bool),
    Tag(String),
    Untag(String),
}

/// Apply `mark` to the session file and save the marks.
pub fn mark(state_dir: &Path, session: &Path, mark: &Mark) -> std::io::Result<()> {
    let mut store = MetaStore::load(state_dir);
    store.update(session, |m| match mark {
        Mark::Pin(on) => m.pinned = *on,
        Mark::Archive(on) => m.archived = *on,
        Mark::Tag(t) => m.tags.push(t.clone()),
        Mark::Untag(t) => m.tags.retain(|x| x != t),
    });
    store.save()
}

// ----------------------------------------------------------- export / import

/// The session as a file anyone can carry to another machine: the saved-session format
/// itself, which `import` reads back. Secrets are masked in the exported copy.
pub fn export_to(entry: &LibraryEntry, dest: &Path) -> std::io::Result<()> {
    if dest.exists() {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{} exists already", dest.display())));
    }
    let mut value = entry.saved.to_json();
    redact_export(&mut value);
    write_atomic(dest, &value.to_string())
}

/// Mask strings before serializing, so escaping cannot hide secrets or break JSON.
fn redact_export(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => *text = crate::shared::domain::redact::redact(text),
        serde_json::Value::Array(values) => values.iter_mut().for_each(redact_export),
        serde_json::Value::Object(values) => values.values_mut().for_each(redact_export),
        _ => {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Unreadable(String),
    NotASession(String),
    Empty,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(m) => write!(f, "cannot read the file: {m}"),
            Self::NotASession(m) => write!(f, "that is not a jev session file: {m}"),
            Self::Empty => f.write_str("the session has no turns"),
        }
    }
}

/// Add an exported session to `project_dir`'s sessions (it becomes resumable there, and
/// keeps its time). Returns the new session file.
pub fn import_file(state_dir: &Path, project_dir: &Path, source: &Path, now_ms: u64) -> Result<PathBuf, ImportError> {
    let text = std::fs::read_to_string(source).map_err(|e| ImportError::Unreadable(e.to_string()))?;
    let saved = SavedSession::parse(&text).map_err(ImportError::NotASession)?;
    adopt(state_dir, project_dir, saved, now_ms)
}

/// Store `saved` as a session of `project_dir`.
pub fn adopt(state_dir: &Path, project_dir: &Path, mut saved: SavedSession, now_ms: u64) -> Result<PathBuf, ImportError> {
    if saved.sessions.turns() == 0 {
        return Err(ImportError::Empty);
    }
    saved.project = session_file::project_key(project_dir);
    // Another machine's CLI sessions mean nothing here.
    saved.sessions = strip_cli_ids(saved.sessions);
    let path = session_file::new_session_file(state_dir, project_dir, now_ms);
    write_atomic(&path, &saved.render()).map_err(|e| ImportError::Unreadable(e.to_string()))?;
    Ok(path)
}

/// The same dialogue with no provider CLI session ids and nobody marked as caught up, so
/// the first turn on any provider is handed the whole conversation.
fn strip_cli_ids(old: Sessions) -> Sessions {
    let mut fresh = Sessions::new();
    fresh.set_name(old.name());
    for turn in old.log() {
        fresh.push_unseen(turn.clone());
    }
    fresh
}

// ------------------------------------------------------------ disk and clean

/// What the sessions take on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskUse {
    pub sessions: usize,
    pub bytes: u64,
    pub pinned: usize,
    pub archived: usize,
    /// `(project, sessions, bytes)`, biggest first.
    pub by_project: Vec<(String, usize, u64)>,
}

pub fn disk_use(entries: &[LibraryEntry]) -> DiskUse {
    let mut out = DiskUse::default();
    for e in entries {
        out.sessions += 1;
        out.bytes += e.bytes;
        out.pinned += usize::from(e.meta.pinned);
        out.archived += usize::from(e.meta.archived);
        match out.by_project.iter_mut().find(|(p, ..)| *p == e.saved.project) {
            Some((_, n, b)) => {
                *n += 1;
                *b += e.bytes;
            }
            None => out.by_project.push((e.saved.project.clone(), 1, e.bytes)),
        }
    }
    out.by_project.sort_by_key(|b| std::cmp::Reverse(b.2));
    out
}

/// `12.3 KB`.
pub fn human_bytes(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / f64::from(1 << 20)),
        b if b >= 1 << 10 => format!("{:.1} KB", b as f64 / f64::from(1 << 10)),
        b => format!("{b} B"),
    }
}

/// Delete sessions last saved more than `older_than_days` ago, and every session file that
/// can no longer be read. Pinned sessions are never deleted. Returns how many files went
/// and how many bytes they were.
pub fn clean(state_dir: &Path, older_than_days: u64, now_ms: u64) -> (usize, u64) {
    let mut meta = MetaStore::load(state_dir);
    let max_age_ms = older_than_days.saturating_mul(DAY_MS);
    let readable: std::collections::HashSet<PathBuf> = scan_all(state_dir).into_iter().map(|e| e.path).collect();
    let mut removed = (0, 0);
    let all_files = session_dirs(state_dir).into_iter().flat_map(|p| {
        if p.is_dir() {
            std::fs::read_dir(p).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())).collect()
        } else {
            vec![p]
        }
    });
    for path in all_files.filter(|p| p.extension().is_some_and(|x| x == "json")) {
        if meta.is_pinned(&path) {
            continue;
        }
        let old = SavedSession::parse(&std::fs::read_to_string(&path).unwrap_or_default())
            .map_or(true, |s| now_ms.saturating_sub(s.saved_ms) > max_age_ms || s.sessions.turns() == 0);
        if old || !readable.contains(&path) {
            let bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
            if std::fs::remove_file(&path).is_ok() {
                meta.forget(&path);
                removed = (removed.0 + 1, removed.1 + bytes);
            }
        }
    }
    let _ = meta.save();
    removed
}

// --------------------------------------------------------------- migration

/// Sessions saved by 0.2.0 and earlier sit as one file per project, `<hash>.json`, next to
/// the per-project folders. Move each into its project's folder (named by its save time)
/// so they list, search and prune like the rest. A file that cannot be read stays where it
/// is. Returns how many were moved; running it again moves nothing.
pub fn migrate_legacy(state_dir: &Path) -> usize {
    let root = state_dir.join("sessions");
    let mut moved = 0;
    for path in std::fs::read_dir(&root).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())) {
        if !path.is_file() || path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(saved) = std::fs::read_to_string(&path).ok().and_then(|t| SavedSession::parse(&t).ok()) else { continue };
        let dest_dir = session_file::project_sessions_dir(state_dir, Path::new(&saved.project));
        let dest = dest_dir.join(format!("{}-legacy.json", saved.saved_ms));
        let written = dest.exists() || write_atomic(&dest, &saved.render()).is_ok();
        if written && std::fs::remove_file(&path).is_ok() {
            moved += 1;
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::domain::session::Sessions;
    use crate::session::domain::turn_record::TurnRecord;

    const NOW: u64 = 100 * DAY_MS;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jev-lib-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn turn(p: ProviderId, prompt: &str, reply: &str, tools: &[&str]) -> TurnRecord {
        TurnRecord { prompt: prompt.into(), provider: p, model: "m".into(), reply: reply.into(), tools: tools.iter().map(|s| s.to_string()).collect(), ok: true }
    }

    fn put(state: &Path, project: &Path, at_days_ago: u64, turns: Vec<TurnRecord>) -> PathBuf {
        let mut sessions = Sessions::new();
        for t in turns {
            sessions.record(t);
        }
        let saved = SavedSession { project: session_file::project_key(project), saved_ms: NOW - at_days_ago * DAY_MS, sessions, current: None };
        let path = session_file::new_session_file(state, project, saved.saved_ms);
        write_atomic(&path, &saved.render()).unwrap();
        path
    }

    fn titles(hits: &[Hit]) -> Vec<String> {
        hits.iter().map(|h| h.entry.title()).collect()
    }

    fn library(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let state = temp(name);
        let (a, b) = (state.join("proj-a"), state.join("proj-b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        put(&state, &a, 1, vec![turn(ProviderId::Claude, "fix the lexer", "Done, lexer fixed.", &["Edit src/lexer.rs"])]);
        put(&state, &a, 20, vec![turn(ProviderId::Codex, "write docs", "Docs written about the parser.", &[])]);
        put(&state, &b, 3, vec![turn(ProviderId::Codex, "parser bug", "Found it in tokens.", &["Edit src/parser.rs"])]);
        (state, a, b)
    }

    #[test]
    fn search_finds_words_in_replies_across_projects_and_shows_the_line() {
        let (state, ..) = library("words");
        let hits = search(scan_all(&state), &Query::parse("parser").unwrap(), NOW);
        assert_eq!(titles(&hits), ["parser bug", "write docs"]);
        assert!(hits[1].snippet.as_deref().unwrap().contains("Docs written about the parser"), "{:?}", hits[1].snippet);
        assert!(search(scan_all(&state), &Query::parse("parser lexer").unwrap(), NOW).is_empty());
    }

    #[test]
    fn a_line_whose_lowercase_is_longer_does_not_break_the_snippet() {
        // U+0130 lowercases to three bytes, so an offset found in the copy is not one of the line.
        let state = temp("fold");
        let proj = state.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        put(&state, &proj, 1, vec![turn(ProviderId::Claude, "ask", "\u{130}\u{130}\u{130} foo is here", &[])]);
        let hits = search(scan_all(&state), &Query::parse("foo").unwrap(), NOW);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.as_deref().unwrap().contains("foo"), "{:?}", hits[0].snippet);
        // Longer than the lead-in, so a character count off by one per U+0130 would skip past it.
        let long = format!("{} foo {}", "\u{130}".repeat(40), "z".repeat(200));
        put(&state, &proj, 2, vec![turn(ProviderId::Claude, "ask", &long, &[])]);
        let hits = search(scan_all(&state), &Query::parse("foo").unwrap(), NOW);
        assert!(hits.iter().all(|h| h.snippet.as_deref().unwrap().contains("foo")), "{:?}", hits.iter().map(|h| &h.snippet).collect::<Vec<_>>());
    }

    #[test]
    fn a_word_ending_in_capital_sigma_is_found_in_the_snippet() {
        let state = temp("sigma");
        let proj = state.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let line = format!("{} ΟΔΟΣ {}", "a".repeat(60), "b".repeat(100));
        put(&state, &proj, 1, vec![turn(ProviderId::Claude, "ask", &line, &[])]);
        let hits = search(scan_all(&state), &Query::parse("ΟΔΟΣ").unwrap(), NOW);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.as_deref().unwrap().contains("ΟΔΟΣ"), "{:?}", hits[0].snippet);
        // Not final before an apostrophe and a letter (σ), so only the later one matches.
        let line = format!("ΟΔΟΣ'Α {} ΟΔΟΣ end", "a".repeat(100));
        put(&state, &proj, 2, vec![turn(ProviderId::Claude, "ask", &line, &[])]);
        let hits = search(scan_all(&state), &Query::parse("ΟΔΟΣ").unwrap(), NOW);
        let later: Vec<_> = hits.iter().filter_map(|h| h.snippet.as_deref()).filter(|s| s.contains("end")).collect();
        assert!(later.len() == 1 && later[0].starts_with("..."), "{later:?}");
    }

    #[test]
    fn filters_narrow_by_provider_file_date_and_project() {
        let (state, ..) = library("filters");
        let run = |q: &str| titles(&search(scan_all(&state), &Query::parse(q).unwrap(), NOW));
        assert_eq!(run("provider:codex"), ["parser bug", "write docs"]);
        assert_eq!(run("file:lexer.rs"), ["fix the lexer"]);
        assert_eq!(run("since:7d"), ["fix the lexer", "parser bug"]);
        assert_eq!(run("project:proj-b"), ["parser bug"]);
        assert_eq!(run("provider:codex since:1w parser"), ["parser bug"]);
        assert!(Query::parse("provider:nobody").is_err());
        assert!(Query::parse("since:soon").is_err());
        for odd in ["since:7日", "since:3д", "since:é", "since:"] {
            assert!(Query::parse(odd).is_err(), "{odd}");
        }
    }

    #[test]
    fn enormous_since_filters_include_all_sessions() {
        let (state, ..) = library("huge-since");
        for filter in ["since:99999999999999d", "since:18446744073709551615d", "since:99999999999999w", "since:18446744073709551615w"] {
            let query = Query::parse(filter).unwrap();
            assert_eq!(query.since_ms, Some(u64::MAX));
            assert_eq!(search(scan_all(&state), &query, NOW).len(), 3);
        }
    }

    #[test]
    fn pinned_come_first_archived_hide_and_tags_filter() {
        let (state, ..) = library("marks");
        let all = scan_all(&state);
        let docs = all.iter().find(|e| e.title() == "write docs").unwrap().path.clone();
        let lexer = all.iter().find(|e| e.title() == "fix the lexer").unwrap().path.clone();
        mark(&state, &docs, &Mark::Pin(true)).unwrap();
        mark(&state, &lexer, &Mark::Archive(true)).unwrap();
        mark(&state, &docs, &Mark::Tag("docs".into())).unwrap();
        let run = |q: &str| titles(&search(scan_all(&state), &Query::parse(q).unwrap(), NOW));
        assert_eq!(run(""), ["write docs", "parser bug"]);
        assert_eq!(run("archived"), ["fix the lexer"]);
        assert_eq!(run("all").len(), 3);
        assert_eq!(run("tag:docs"), ["write docs"]);
        assert_eq!(run("pinned"), ["write docs"]);
        mark(&state, &docs, &Mark::Untag("docs".into())).unwrap();
        assert!(run("tag:docs").is_empty());
    }

    #[test]
    fn an_export_imports_into_another_project_and_resumes_there() {
        let (state, a, _) = library("export");
        let entry = scan_all(&state).into_iter().find(|e| e.title() == "fix the lexer").unwrap();
        let file = state.join("out.json");
        export_to(&entry, &file).unwrap();
        assert!(export_to(&entry, &file).is_err(), "never overwrites");
        let other = state.join("proj-c");
        std::fs::create_dir_all(&other).unwrap();
        let moved = import_file(&state, &other, &file, NOW).unwrap();
        let listed = session_file::list_sessions(&state, &other, NOW, None);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, moved);
        assert_eq!(listed[0].title, "fix the lexer");
        // The original stays where it was.
        // (The 20-day-old one is past the 14 days `/resume` offers.)
        assert_eq!(session_file::list_sessions(&state, &a, NOW, None).len(), 1);
        assert!(matches!(import_file(&state, &other, &state.join("nope.json"), NOW), Err(ImportError::Unreadable(_))));
        std::fs::write(state.join("junk.json"), "{}").unwrap();
        assert!(matches!(import_file(&state, &other, &state.join("junk.json"), NOW), Err(ImportError::NotASession(_))));
    }

    #[test]
    fn export_masks_secrets_without_changing_the_stored_session() {
        let state = temp("export-secrets");
        let project = state.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let original_path = put(&state, &project, 1, vec![turn(
            ProviderId::Claude,
            r#"use {"password": "hunter2hunter2"}"#,
            "SECRET = replysecret",
            &["Bash curl https://alice:toolpassword@example.com", "Read xai-abcdefghijklmnopqrstuvwx"],
        )]);
        let original = std::fs::read_to_string(&original_path).unwrap();
        let entry = scan_all(&state).into_iter().next().unwrap();
        let output = state.join("export.json");
        export_to(&entry, &output).unwrap();
        let exported = std::fs::read_to_string(&output).unwrap();
        for secret in ["hunter2", "replysecret", "toolpassword", "xai-abcdefghijklmnopqrstuvwx"] {
            assert!(!exported.contains(secret), "{exported}");
            assert!(original.contains(secret));
        }
        let saved = SavedSession::parse(&exported).unwrap();
        let turn = &saved.sessions.log()[0];
        assert!(turn.prompt.contains("[REDACTED]"));
        assert!(turn.reply.contains("[REDACTED]"));
        assert!(turn.tools.iter().all(|tool| tool.contains("[REDACTED]")));
        assert_eq!(std::fs::read_to_string(original_path).unwrap(), original);
        assert!(entry.saved.sessions.log()[0].prompt.contains("hunter2"));
    }

    #[test]
    fn disk_use_counts_by_project_and_clean_spares_pinned() {
        let (state, ..) = library("clean");
        let usage = disk_use(&scan_all(&state));
        assert_eq!((usage.sessions, usage.by_project.len()), (3, 2));
        let old = scan_all(&state).into_iter().find(|e| e.title() == "write docs").unwrap().path;
        mark(&state, &old, &Mark::Pin(true)).unwrap();
        std::fs::write(state.join("sessions").join("garbage.json"), "{").unwrap();
        // 10 days: the 20-day-old session is pinned (kept), the garbage file goes.
        let (n, _) = clean(&state, 10, NOW);
        assert_eq!(n, 1);
        assert_eq!(scan_all(&state).len(), 3);
        mark(&state, &old, &Mark::Pin(false)).unwrap();
        assert_eq!(clean(&state, 10, NOW).0, 1);
        assert_eq!(scan_all(&state).len(), 2);
    }

    #[test]
    fn clean_with_enormous_age_keeps_readable_nonempty_sessions() {
        let (state, ..) = library("huge-clean");
        for days in [999_999_999_999, u64::MAX] {
            assert_eq!(clean(&state, days, NOW), (0, 0));
            assert_eq!(scan_all(&state).len(), 3);
            assert_eq!(clean(&state, days, u64::MAX), (0, 0));
            assert_eq!(scan_all(&state).len(), 3);
        }
    }

    #[test]
    fn legacy_single_files_move_into_their_project_folder_once() {
        let state = temp("legacy");
        let project = state.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let mut sessions = Sessions::new();
        sessions.record(turn(ProviderId::Claude, "old work", "ok", &[]));
        let saved = SavedSession { project: session_file::project_key(&project), saved_ms: NOW - DAY_MS, sessions, current: None };
        let legacy = session_file::legacy_session_file(&state, &project);
        write_atomic(&legacy, &saved.render()).unwrap();
        std::fs::write(state.join("sessions").join("zzz.json"), "not a session").unwrap();
        assert_eq!(migrate_legacy(&state), 1);
        assert!(!legacy.exists());
        let listed = session_file::list_sessions(&state, &project, NOW, None);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "old work");
        assert_eq!(migrate_legacy(&state), 0);
    }

    #[test]
    fn rendered_hits_show_marks_and_the_matching_line() {
        let (state, a, _) = library("render");
        let hits = search(scan_all(&state), &Query::parse("lexer").unwrap(), NOW);
        let text = render_hits(&hits, NOW, &session_file::project_key(&a), Style::off()).join("\n");
        assert!(text.contains("  1. fix the lexer") && text.contains("24h ago") && text.contains("claude"), "{text}");
        assert!(!text.contains("proj-a"), "the current project is not repeated: {text}");
        let elsewhere = render_hits(&hits, NOW, "/other", Style::off()).join("\n");
        assert!(elsewhere.contains("proj-a"), "{elsewhere}");
    }
}
