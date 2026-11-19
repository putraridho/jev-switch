//! The facts behind each screen of `jev-code`, taken from the engine and its runtime and
//! handed to the formatters in [`crate::uifmt`]. Everything here is a pure function of its
//! arguments (the clock and the home directory are passed in), so the binary and the REPL
//! only decide WHEN to print and each mapping is a unit test.

use std::path::Path;

use crate::availability::{Availability, State};
use crate::backend::ProviderId;
use crate::catalog;
use crate::changes::{ChangeStatus, ChangeSummary};
use crate::doctor;
use crate::handoff::SessionInfo;
use crate::providers::{format_left, BlockReason, Runtime, ALL_PROVIDERS};
use crate::routing::Pin;
use crate::turn::{Engine, TurnReport, TurnStatus};
use crate::uifmt::{
    self, AvailRow, CompleteCtx, DoctorItem, FileChange, ProviderStatus, ReadyProvider, StatusFacts, Style, SummaryFacts,
};

// ------------------------------------------------------------------ providers

fn status_of(state: State) -> ProviderStatus {
    match state {
        State::Ready => ProviderStatus::Ready,
        State::NotInstalled => ProviderStatus::NotInstalled,
        State::NotLoggedIn => ProviderStatus::NotLoggedIn,
        State::NoKey => ProviderStatus::NoKey,
        State::Unknown => ProviderStatus::Unknown,
    }
}

/// One row per provider as `/providers` and the banner show it: the probe, then what has
/// happened since (a usage-limit cooldown, a login that failed, a directory it refuses).
pub fn avail_rows(rt: &Runtime, now_ms: u64) -> Vec<AvailRow> {
    rt.avail.iter().map(|a| avail_row(a, rt, now_ms)).collect()
}

fn avail_row(a: &Availability, rt: &Runtime, now_ms: u64) -> AvailRow {
    let mut row = AvailRow::new(a.provider.as_str(), status_of(a.state));
    row.plan = a.plan.clone();
    row.models = a.models.clone();
    row.detail = a.detail.clone();
    row.verified = a.verified;
    row.cooldown = rt.cooldown_left(a.provider, now_ms).map(format_left);
    if a.provider == ProviderId::Gemini {
        row.via = Some(crate::providers::gemini_via().to_string());
    }
    match rt.blocked.get(&a.provider) {
        Some(BlockReason::Auth) => {
            row.status = ProviderStatus::NotLoggedIn;
            row.detail = "its login failed this session; log in again, then /providers refresh".into();
        }
        Some(BlockReason::UnusableHere) => {
            row.status = ProviderStatus::Unknown;
            row.detail = "can't run in this directory (/new clears it)".into();
        }
        None => {}
    }
    row
}

/// The models routing may pick right now, as `provider:model`, in catalog order.
fn usable_models(rt: &Runtime, now_ms: u64) -> Vec<(ProviderId, &'static str)> {
    let usable = rt.usable(now_ms);
    catalog::models().iter().filter(|m| usable.contains(m)).map(|m| (m.provider, m.id)).collect()
}

/// What Tab completion may offer: the providers and models that can run now, and the files
/// the last turn changed.
pub fn complete_ctx(engine: &Engine, now_ms: u64) -> CompleteCtx {
    let usable = usable_models(&engine.runtime, now_ms);
    let mut ready_providers: Vec<String> = Vec::new();
    for (p, _) in &usable {
        if !ready_providers.iter().any(|r| r == p.as_str()) {
            ready_providers.push(p.to_string());
        }
    }
    CompleteCtx {
        ready_providers,
        models: usable.iter().map(|(p, m)| format!("{p}:{m}")).collect(),
        changed_files: engine.last_changes().map(|c| c.files.iter().map(|f| f.path.clone()).collect()).unwrap_or_default(),
        skills: skill_choices(engine.dir()),
    }
}

/// The skills of this project and the user, for `/skill` completion.
fn skill_choices(project: &std::path::Path) -> Vec<crate::uifmt::SkillChoice> {
    let lib = crate::ext::library::Library::discover(&crate::ext::Places::from_env(project));
    lib.of_kind(crate::ext::library::Kind::Skill)
        .map(|e| crate::uifmt::SkillChoice { name: e.name.clone(), description: e.meta.description.clone(), hint: e.meta.hint.clone() })
        .collect()
}

/// The facts of `/status`.
pub fn status_facts(engine: &Engine, last_turn: Option<&SummaryFacts>, home: Option<&str>, now_ms: u64) -> StatusFacts {
    let rt = &engine.runtime;
    let usable = usable_models(rt, now_ms);
    let ready = ALL_PROVIDERS
        .iter()
        .filter(|p| usable.iter().any(|(u, _)| u == *p))
        .map(|p| ReadyProvider {
            provider: p.to_string(),
            plan: rt.avail.iter().find(|a| a.provider == *p).and_then(|a| a.plan.clone()),
            plan_used: rt.plan_utilization(*p),
        })
        .collect();
    let cooldowns = ALL_PROVIDERS
        .iter()
        .filter_map(|p| Some((p.to_string(), format_left(rt.cooldown_left(*p, now_ms)?))))
        .collect();
    StatusFacts {
        project: display_path(engine.dir(), home),
        routing_on: engine.enabled(),
        pin: engine.pin().to_string(),
        orchestrate: engine.orchestrate(),
        ready,
        cooldowns,
        last_turn: last_turn.cloned(),
        turns: engine.sessions.turns(),
    }
}

// ---------------------------------------------------------------------- turns

fn change_letter(s: ChangeStatus) -> char {
    match s {
        ChangeStatus::Added => 'A',
        ChangeStatus::Modified => 'M',
        ChangeStatus::Deleted => 'D',
        ChangeStatus::Renamed => 'R',
        ChangeStatus::ModifiedUntracked => 'U',
    }
}

/// The changed files as the formatter wants them.
pub fn file_changes(summary: &ChangeSummary) -> Vec<FileChange> {
    summary
        .files
        .iter()
        .map(|f| FileChange { status: change_letter(f.status), path: f.path.clone(), added: f.added, removed: f.removed })
        .collect()
}

/// The facts of the one-line summary after a turn. `elapsed_ms` is the wall time the
/// caller measured; the plan percentage is the last one the answering provider reported.
pub fn summary_facts(report: &TurnReport, elapsed_ms: u64, rt: &Runtime) -> SummaryFacts {
    let usage = report.usage.unwrap_or_default();
    SummaryFacts {
        ok: report.status == TurnStatus::Answered,
        cancelled: report.status == TurnStatus::Cancelled,
        provider: report.provider.map(|p| p.to_string()).unwrap_or_default(),
        model: report.model.clone().unwrap_or_default(),
        elapsed_ms,
        in_tokens: usage.input_tokens,
        cached_tokens: usage.cached_input_tokens,
        out_tokens: usage.output_tokens,
        plan: report.provider.and_then(|p| Some((p.to_string(), rt.plan_utilization(p)?))),
        files_changed: report.changes.as_ref().map(|c| {
            let (added, removed) = c.totals();
            (c.files.len(), added, removed)
        }),
    }
}

/// The process exit code of a one-shot turn: 0 answered, 130 stopped (the shell's
/// convention for Ctrl-C), 1 for anything else.
pub fn exit_code(status: TurnStatus) -> i32 {
    match status {
        TurnStatus::Answered => 0,
        TurnStatus::Cancelled => 130,
        TurnStatus::NoRoute | TurnStatus::Failed => 1,
    }
}

// -------------------------------------------------------------------- startup

/// `dir` with the home directory shown as `~`.
pub fn display_path(dir: &Path, home: Option<&str>) -> String {
    let full = dir.to_string_lossy();
    match home.map(|h| h.trim_end_matches('/')).filter(|h| !h.is_empty()) {
        Some(h) if full == h => "~".to_string(),
        Some(h) => full.strip_prefix(&format!("{h}/")).map_or_else(|| full.to_string(), |rest| format!("~/{rest}")),
        None => full.to_string(),
    }
}

/// The session header, one line: `jev-code 0.2.0 · claude, codex ready · ~/proj`. Providers
/// are named in words (no symbol to learn); the full table lives behind `/providers`, and
/// with nothing ready the line says where to look.
pub fn banner(dir: &Path, home: Option<&str>, routing_on: bool, rows: &[AvailRow], style: Style, width: usize) -> String {
    let width = width.max(uifmt::MIN_WIDTH);
    let ready: Vec<&str> = rows.iter().filter(|r| r.status == ProviderStatus::Ready && r.cooldown.is_none()).map(|r| r.provider.as_str()).collect();
    let cooling: Vec<&str> = rows.iter().filter(|r| r.status == ProviderStatus::Ready && r.cooldown.is_some()).map(|r| r.provider.as_str()).collect();
    let mut providers = if ready.is_empty() { style.yellow("no provider ready · /doctor") } else { format!("{} ready", ready.join(", ")) };
    if !cooling.is_empty() {
        providers.push_str(&style.dim(&format!(" · {} cooling down", cooling.join(", "))));
    }
    let routing = if routing_on { String::new() } else { format!(" {} {}", style.dim("·"), style.yellow("Claude only")) };
    let brand = format!("{} {}", style.bold(&style.cyan("jev-code")), style.dim(env!("CARGO_PKG_VERSION")));
    let sep = style.dim("·");
    // What can run comes before the folder: a long path is what gets cut, never the providers.
    uifmt::truncate_to_width(&format!("{brand} {sep} {providers}{routing} {sep} {}", display_path(dir, home)), width)
}

/// "just now", "5m ago", "2h ago", "3d ago".
pub fn human_ago(ms: u64) -> String {
    const MIN: u64 = 60_000;
    match ms / MIN {
        0 => "just now".to_string(),
        m @ 1..=59 => format!("{m}m ago"),
        m if m < 48 * 60 => format!("{}h ago", m / 60),
        m => format!("{}d ago", m / (24 * 60)),
    }
}

/// The dim line offering the saved session: `previous session: 12 turns, 2h ago · /resume`.
pub fn previous_session_line(info: &SessionInfo, now_ms: u64, style: Style) -> String {
    let turns = if info.turns == 1 { "1 turn".to_string() } else { format!("{} turns", info.turns) };
    let ago = human_ago(now_ms.saturating_sub(info.last_used_ms));
    style.dim(&format!("previous session: {turns}, {ago} · /resume"))
}

/// What `/resume` (or `-c`) says after loading a session.
pub fn resumed_line(info: &SessionInfo, now_ms: u64, style: Style) -> String {
    let turns = if info.turns == 1 { "1 turn".to_string() } else { format!("{} turns", info.turns) };
    let last = info.current.as_ref().map_or(String::new(), |(p, m)| format!(", last on {p} · {m}"));
    let ago = human_ago(now_ms.saturating_sub(info.last_used_ms));
    format!("{} resumed {}", style.green("✓"), style.dim(&format!("{turns}{last}, {ago}")))
}

/// The marker file whose presence means the first-run hint was shown on this machine.
pub const FIRST_RUN_MARKER: &str = "first-run-done";

/// The first-run hint, once per machine: the text when there is something to say and it was
/// not shown before, and the marker is written so it is never shown again. A machine where
/// the marker cannot be written gets no hint rather than a hint on every start. `/doctor`
/// always has the full story.
pub fn first_run_hint_once(avail: &[Availability], state_dir: Option<&Path>) -> Option<String> {
    let dir = state_dir?;
    let marker = dir.join(FIRST_RUN_MARKER);
    if marker.exists() {
        return None;
    }
    let hint = doctor::first_run_hint(avail)?;
    std::fs::write(&marker, "shown\n").ok()?;
    Some(hint)
}

// --------------------------------------------------------------------- prompt

/// What the prompt says about routing: the pin (`codex`, `claude:claude-opus-5-5`), or
/// `claude only` when routing is off; `None` when the router is free to choose.
pub fn prompt_label(pin: &Pin, routing_on: bool) -> Option<String> {
    match pin {
        Pin::Auto => (!routing_on).then(|| "claude only".to_string()),
        other => Some(other.to_string()),
    }
}

/// `auto (accepts edits and safe commands, asks about the rest)`.
pub fn mode_line(mode: crate::mode::Mode) -> String {
    format!("{mode} ({})", mode.about())
}

/// The prompt as plain text, which is what the line editor measures: `› ` or `[codex] › `.
pub fn plain_prompt(label: Option<&str>) -> String {
    match label {
        Some(l) => format!("[{l}] › "),
        None => "› ".to_string(),
    }
}

/// The same prompt coloured: the label in the provider's colour, the arrow bold.
pub fn styled_prompt(label: Option<&str>, style: Style) -> String {
    let arrow = style.cyan(&style.bold("›"));
    match label {
        Some(l) => {
            let provider = l.split(':').next().unwrap_or(l);
            let tag = if provider.parse::<ProviderId>().is_ok() {
                uifmt::Theme::new(style).provider(provider, &format!("[{l}]"))
            } else {
                style.dim(&format!("[{l}]"))
            };
            format!("{tag} {arrow} ")
        }
        None => format!("{arrow} "),
    }
}

// ---------------------------------------------------------------------- doctor

fn doctor_item(c: &doctor::Check) -> DoctorItem {
    DoctorItem {
        severity: match c.severity {
            doctor::Severity::Ok => uifmt::Severity::Ok,
            doctor::Severity::Info => uifmt::Severity::Info,
            doctor::Severity::Warn => uifmt::Severity::Warn,
            doctor::Severity::Problem => uifmt::Severity::Fail,
        },
        title: c.title.clone(),
        detail: c.detail.clone(),
        fix: c.fix.clone(),
    }
}

/// Whether the report has a Problem: the `jev-code doctor` exit code is 1 then.
pub fn has_problem(report: &doctor::Report) -> bool {
    report.checks.iter().any(|c| c.severity == doctor::Severity::Problem)
}

/// The doctor's screen: findings with the worst first, what routing can do now, and a
/// closing count.
pub fn doctor_text(report: &doctor::Report, style: Style, width: usize) -> String {
    let items: Vec<DoctorItem> = report.checks.iter().map(doctor_item).collect();
    let mut out = uifmt::doctor_report(&items, style, width);
    out.push_str(&format!("\n\n{}\n{}", report.routing_note, uifmt::doctor_summary(&items, style)));
    out
}

// ------------------------------------------------------------------------ pager

/// The pager to start for `$PAGER`: its program and arguments, or `less -R` (which keeps
/// colours) when it is unset or blank.
pub fn pager_command(pager_env: Option<&str>) -> (String, Vec<String>) {
    let mut words = pager_env.unwrap_or("").split_whitespace().map(String::from);
    match words.next() {
        Some(program) => (program, words.collect()),
        None => ("less".to_string(), vec!["-R".to_string()]),
    }
}

/// Whether `text` goes through a pager: only on a terminal, and only when it would scroll
/// off the screen. Never when there is no terminal, so a pipe is never blocked.
pub fn should_page(text: &str, rows: Option<usize>, stdout_is_tty: bool) -> bool {
    stdout_is_tty && text.lines().count() > rows.unwrap_or(24).saturating_sub(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Usage;
    use crate::changes::FileChange as GitChange;
    use std::path::PathBuf;
    use ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

    const NOW: u64 = 1_760_000_000_000;

    fn av(provider: ProviderId, state: State) -> Availability {
        Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
    }

    fn runtime() -> Runtime {
        Runtime::for_tests(vec![
            av(Claude, State::Ready),
            av(DeepSeek, State::NoKey),
            av(Codex, State::Ready),
            av(Gemini, State::NotInstalled),
            av(Grok, State::NotLoggedIn),
        ])
    }

    fn engine(rt: Runtime) -> Engine {
        use crate::router::Router;
        use crate::typesafe::TypeSafe;
        Engine::new(rt, Router::new(TypeSafe::with_url("k", "http://127.0.0.1:1"), 1.0), PathBuf::from("/home/me/proj"))
            .with_turn_log(None)
    }

    #[test]
    fn rows_carry_the_probe_and_what_happened_since() {
        let mut rt = runtime();
        rt.cool_down(Codex, Some("1760003600"), NOW);
        rt.blocked.insert(Grok, BlockReason::Auth);
        rt.blocked.insert(Claude, BlockReason::UnusableHere);
        let rows = avail_rows(&rt, NOW);
        assert_eq!(rows.iter().map(|r| r.provider.as_str()).collect::<Vec<_>>(), ["claude", "deepseek", "codex", "gemini", "grok"]);
        assert_eq!(rows[1].status, ProviderStatus::NoKey);
        assert_eq!(rows[2].status, ProviderStatus::Ready);
        assert!(rows[2].cooldown.as_deref().is_some_and(|c| c.contains('h') || c.contains('m')), "{:?}", rows[2].cooldown);
        assert_eq!(rows[3].status, ProviderStatus::NotInstalled);
        assert_eq!(rows[4].status, ProviderStatus::NotLoggedIn);
        assert!(rows[4].detail.contains("/providers refresh"));
        assert_eq!(rows[0].status, ProviderStatus::Unknown, "blocked for this directory");
        assert!(rows[0].detail.contains("/new"));
    }

    #[test]
    fn completion_knows_the_usable_providers_models_and_changed_files() {
        let e = engine(runtime());
        let ctx = complete_ctx(&e, NOW);
        assert_eq!(ctx.ready_providers, ["claude", "codex"]);
        assert!(ctx.models.iter().all(|m| m.starts_with("claude:") || m.starts_with("codex:")), "{:?}", ctx.models);
        assert!(ctx.models.contains(&"codex:gpt-6.1-sol".to_string()));
        assert!(ctx.changed_files.is_empty());
    }

    #[test]
    fn status_lists_ready_providers_and_cooldowns() {
        let mut rt = runtime();
        rt.cool_down(Codex, Some("1760003600"), NOW);
        let e = engine(rt);
        let f = status_facts(&e, None, Some("/home/me"), NOW);
        assert_eq!(f.project, "~/proj");
        assert!(f.routing_on && !f.orchestrate);
        assert_eq!(f.pin, "auto");
        assert_eq!(f.ready.iter().map(|r| r.provider.as_str()).collect::<Vec<_>>(), ["claude"], "codex is cooling down");
        assert_eq!(f.cooldowns.len(), 1);
        assert_eq!(f.cooldowns[0].0, "codex");
        assert_eq!(f.turns, 0);
    }

    #[test]
    fn the_summary_comes_from_the_report() {
        let mut rt = runtime();
        rt.note_plan(Claude, crate::backend::PlanUsage { window: "five_hour".into(), utilization: 0.32, resets_at: None });
        let report = TurnReport {
            status: TurnStatus::Answered,
            text: "x".into(),
            message: String::new(),
            provider: Some(Claude),
            model: Some("claude-sonnet-5-5".into()),
            usage: Some(Usage { input_tokens: 45_000, cached_input_tokens: 40_000, output_tokens: 189, context_tokens: None }),
            attempts: 1,
            tool_calls: 0,
            changes: Some(ChangeSummary {
                files: vec![GitChange { path: "a.rs".into(), status: ChangeStatus::Modified, added: 12, removed: 4, old_path: None }],
                before: "b".into(),
                after: "a".into(),
                added_stamps: Default::default(),
                ..ChangeSummary::default()
            }),
        };
        let f = summary_facts(&report, 14_000, &rt);
        assert!(f.ok && !f.cancelled);
        assert_eq!((f.provider.as_str(), f.model.as_str()), ("claude", "claude-sonnet-5-5"));
        assert_eq!((f.in_tokens, f.cached_tokens, f.out_tokens), (45_000, 40_000, 189));
        assert_eq!(f.plan, Some(("claude".to_string(), 0.32)));
        assert_eq!(f.files_changed, Some((1, 12, 4)));

        let stopped = TurnReport { status: TurnStatus::Cancelled, usage: None, changes: None, provider: None, model: None, ..report };
        let f = summary_facts(&stopped, 3000, &rt);
        assert!(f.cancelled && !f.ok && f.plan.is_none() && f.files_changed.is_none());
    }

    #[test]
    fn changed_files_map_to_status_letters() {
        let s = ChangeSummary {
            files: [ChangeStatus::Added, ChangeStatus::Modified, ChangeStatus::Deleted, ChangeStatus::Renamed]
                .into_iter()
                .map(|status| GitChange { path: "p".into(), status, added: 1, removed: 2, old_path: None })
                .collect(),
            before: String::new(),
            after: String::new(),
            added_stamps: Default::default(),
            ..ChangeSummary::default()
        };
        assert_eq!(file_changes(&s).iter().map(|f| f.status).collect::<String>(), "AMDR");
    }

    #[test]
    fn exit_codes_follow_the_shell_conventions() {
        assert_eq!(exit_code(TurnStatus::Answered), 0);
        assert_eq!(exit_code(TurnStatus::Failed), 1);
        assert_eq!(exit_code(TurnStatus::NoRoute), 1);
        assert_eq!(exit_code(TurnStatus::Cancelled), 130);
    }

    #[test]
    fn the_home_directory_is_shown_as_a_tilde() {
        assert_eq!(display_path(Path::new("/home/me/proj"), Some("/home/me")), "~/proj");
        assert_eq!(display_path(Path::new("/home/me"), Some("/home/me/")), "~");
        assert_eq!(display_path(Path::new("/home/meagan/proj"), Some("/home/me")), "/home/meagan/proj");
        assert_eq!(display_path(Path::new("/srv/x"), None), "/srv/x");
    }

    #[test]
    fn the_banner_is_one_line_naming_the_ready_providers_in_words() {
        let rows = avail_rows(&runtime(), NOW);
        let ready: Vec<String> = rows.iter().filter(|r| r.status == ProviderStatus::Ready && r.cooldown.is_none()).map(|r| r.provider.clone()).collect();
        let b = banner(Path::new("/home/me/proj"), Some("/home/me"), true, &rows, Style::off(), 100);
        let v = env!("CARGO_PKG_VERSION");
        assert!(b.starts_with(&format!("jev-code {v} · ")) && b.ends_with(" · ~/proj"), "{b}");
        assert_eq!(b.lines().count(), 1);
        if ready.is_empty() {
            assert!(b.contains("no provider ready · /doctor"), "{b}");
        } else {
            assert!(b.contains(&format!("{} ready", ready.join(", "))), "{b}");
        }
        assert!(!b.contains('●') && !b.contains('✕'), "no symbols to decode: {b}");
        let off = banner(Path::new("/x"), None, false, &rows, Style::off(), 100);
        assert!(off.ends_with("· Claude only · /x"), "{off}");
        let none = banner(Path::new("/x"), None, true, &[], Style::off(), 100);
        assert!(none.contains("no provider ready · /doctor · /x"), "{none}");
        let long = banner(Path::new(&format!("/{}", "deep/".repeat(40))), None, true, &rows, Style::off(), 60);
        assert!(long.contains(" ready") || long.contains("no provider ready"), "a long path never hides the providers: {long}");
        let narrow = banner(Path::new("/a/very/long/project/directory"), None, true, &rows, Style::off(), 40);
        assert!(uifmt::display_width(&narrow) <= 40, "{narrow}");
    }

    #[test]
    fn ages_read_the_way_people_say_them() {
        for (ms, want) in [(0, "just now"), (59_999, "just now"), (60_000, "1m ago"), (59 * 60_000, "59m ago"), (60 * 60_000, "1h ago"), (47 * 3_600_000, "47h ago"), (48 * 3_600_000, "2d ago")] {
            assert_eq!(human_ago(ms), want, "{ms}");
        }
    }

    #[test]
    fn the_previous_session_line_offers_resume() {
        let info = SessionInfo { turns: 12, last_used_ms: NOW - 2 * 3_600_000, providers: vec![Claude], current: Some((Claude, "sonnet".into())) };
        assert_eq!(previous_session_line(&info, NOW, Style::off()), "previous session: 12 turns, 2h ago · /resume");
        let one = SessionInfo { turns: 1, ..info.clone() };
        assert!(previous_session_line(&one, NOW, Style::off()).contains("1 turn,"));
        assert_eq!(resumed_line(&info, NOW, Style::off()), "✓ resumed 12 turns, last on claude · sonnet, 2h ago");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jev-views-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_first_run_hint_is_shown_once_per_machine() {
        let dir = temp_dir("first-run");
        let one_ready = vec![av(Claude, State::Ready), av(Codex, State::NotInstalled)];
        let hint = first_run_hint_once(&one_ready, Some(&dir)).expect("first time");
        assert!(hint.contains("Only one provider is ready"), "{hint}");
        assert!(dir.join(FIRST_RUN_MARKER).exists());
        assert_eq!(first_run_hint_once(&one_ready, Some(&dir)), None, "never again");

        let other = temp_dir("first-run-fine");
        let all_ready = vec![av(Claude, State::Ready), av(Codex, State::Ready)];
        assert_eq!(first_run_hint_once(&all_ready, Some(&other)), None, "nothing to say");
        assert!(!other.join(FIRST_RUN_MARKER).exists(), "the marker is only written after a hint was shown");

        assert_eq!(first_run_hint_once(&one_ready, None), None, "no state directory: no nagging");
        let blocked = dir.join("nope").join("deeper");
        assert_eq!(first_run_hint_once(&one_ready, Some(&blocked)), None, "a marker that cannot be written means no hint");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(other);
    }

    #[test]
    fn the_prompt_shows_the_pin_or_routing_off() {
        use crate::backend::ProviderId::Codex;
        assert_eq!(prompt_label(&Pin::Auto, true), None);
        assert_eq!(prompt_label(&Pin::Auto, false).as_deref(), Some("claude only"));
        assert_eq!(prompt_label(&Pin::Provider(Codex), true).as_deref(), Some("codex"));
        assert_eq!(prompt_label(&Pin::Model(Claude, "claude-opus-5-5".into()), true).as_deref(), Some("claude:claude-opus-5-5"));
        assert_eq!(plain_prompt(None), "› ");
        assert_eq!(plain_prompt(Some("codex")), "[codex] › ");
    }

    #[test]
    fn the_styled_prompt_is_the_plain_one_with_colour() {
        for label in [None, Some("codex"), Some("claude:claude-opus-5-5"), Some("claude only")] {
            assert_eq!(uifmt::strip_ansi(&styled_prompt(label, Style::on())), plain_prompt(label));
            assert_eq!(styled_prompt(label, Style::off()), plain_prompt(label));
        }
        assert!(styled_prompt(Some("codex"), Style::on()).contains("\x1b[32m"), "codex is green");
    }

    #[test]
    fn the_doctor_screen_puts_problems_first_and_ends_with_a_count() {
        let inputs = doctor::Inputs { avail: vec![av(Claude, State::NotLoggedIn)], jev_key_present: false, ..doctor::Inputs::default() };
        let report = doctor::diagnose(&inputs);
        assert!(has_problem(&report));
        let text = doctor_text(&report, Style::off(), 80);
        assert!(text.starts_with("✗ "), "{text}");
        assert!(text.contains("$ claude auth login"), "{text}");
        assert!(text.contains("No provider is ready"), "{text}");
        assert!(text.lines().last().unwrap().contains("problem"), "{text}");

        let fine = doctor::diagnose(&doctor::Inputs { avail: vec![av(Claude, State::Ready), av(Codex, State::Ready)], jev_key_present: true, ..doctor::Inputs::default() });
        assert!(!has_problem(&fine));
    }

    #[test]
    fn the_pager_is_pager_or_less() {
        assert_eq!(pager_command(None), ("less".into(), vec!["-R".into()]));
        assert_eq!(pager_command(Some("  ")), ("less".into(), vec!["-R".into()]));
        assert_eq!(pager_command(Some("bat -p --color=always")), ("bat".into(), vec!["-p".into(), "--color=always".into()]));
        assert_eq!(pager_command(Some("cat")), ("cat".into(), vec![]));
    }

    #[test]
    fn only_long_output_on_a_terminal_is_paged() {
        let long = "x\n".repeat(100);
        let short = "x\n".repeat(5);
        assert!(should_page(&long, Some(40), true));
        assert!(!should_page(&long, Some(40), false), "never without a terminal");
        assert!(!should_page(&short, Some(40), true));
        assert!(should_page(&"x\n".repeat(23), None, true), "an unknown height is taken as 24 rows");
        assert!(!should_page(&"x\n".repeat(22), None, true));
    }
}
