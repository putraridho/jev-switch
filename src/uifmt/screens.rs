//! Whole screens: `/help`, `/status` and the `doctor` report.

use super::commands::{Group, COMMANDS, KEYS, usage};
use super::style::{Style, Theme};
use super::summary::{turn_summary, SummaryFacts};
use super::text::{one_line, plural};
use super::width::{clamp_width, display_width, pad_to_width, truncate_to_width};
use super::wrap::wrap_to_width;

/// The widest the command column of `/help` grows.
const COMMAND_COL_MAX: usize = 28;

/// The `/help` screen for `width` columns: the commands in five groups, then the keys. A
/// description that does not fit wraps at a word, under itself; on a narrow terminal it goes
/// on the line under its command. No row is ever wider than `width`.
pub fn help_text(style: Style, width: usize) -> String {
    let width = width.max(super::MIN_WIDTH);
    let mut lines: Vec<String> = vec![style.bold("Commands & shortcuts")];
    lines.extend(super::wrap_to_width("Type / to pick a command from a menu; Tab completes, Enter runs.", width).iter().map(|l| style.dim(l)));
    lines.push(String::new());
    // Capped, so one long synopsis (`/diff ...`) does not push every description off to the right.
    let command_col = (COMMANDS.iter().map(|c| display_width(&usage(c))).max().unwrap_or(0) + 2).min(COMMAND_COL_MAX);
    let key_col = KEYS.iter().map(|(k, _)| display_width(k)).max().unwrap_or(0) + 2;
    let rows = |pairs: Vec<(String, &str)>, col: usize, lines: &mut Vec<String>| {
        // Side by side while the description keeps 30 columns; else under the command.
        let side = width >= 2 + col + 30;
        let (indent, room) = if side { (2 + col, width - 2 - col) } else { (4, width - 4) };
        for (key, about) in pairs {
            let wrapped = super::wrap_to_width(about, room);
            if side && display_width(&key) + 2 > col {
                // Too wide for the column: it gets a row, its description starts under it.
                lines.push(truncate_to_width(&format!("  {}", style.cyan(&key)), width));
                lines.extend(wrapped.iter().map(|part| format!("{}{part}", " ".repeat(indent))));
            } else if side {
                for (i, part) in wrapped.iter().enumerate() {
                    let lead = if i == 0 { format!("  {}", style.cyan(&pad_to_width(&key, col))) } else { " ".repeat(indent) };
                    lines.push(format!("{lead}{part}"));
                }
            } else {
                lines.push(truncate_to_width(&format!("  {}", style.cyan(&key)), width));
                lines.extend(wrapped.iter().map(|part| format!("{}{part}", " ".repeat(indent))));
            }
        }
    };
    for (group, title) in [(Group::Model, "Model & routing"), (Group::Session, "Session"), (Group::Context, "Context & output"), (Group::Project, "Project"), (Group::Info, "Info")] {
        lines.push(style.bold(&style.cyan(title)));
        rows(COMMANDS.iter().filter(|c| c.group == group).map(|c| (usage(c), c.about)).collect(), command_col, &mut lines);
        lines.push(String::new());
    }
    lines.push(style.bold("Keys"));
    rows(KEYS.iter().map(|(k, what)| (k.to_string(), *what)).collect(), key_col, &mut lines);
    lines.join("\n")
}

// -------------------------------------------------------------------- status

/// A provider that is ready, for `/status`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadyProvider {
    pub provider: String,
    pub plan: Option<String>,
    /// Fraction of its plan window used (0..1), when known.
    pub plan_used: Option<f64>,
    /// Account credit left for a pay-per-token provider (`$12.34 left`), when known.
    pub credit: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusFacts {
    pub project: String,
    pub routing_on: bool,
    /// How the pin prints: `auto`, `codex`, `claude:claude-opus-5-5`.
    pub pin: String,
    pub orchestrate: bool,
    pub ready: Vec<ReadyProvider>,
    /// (provider, time left) for each provider cooling down after a limit.
    pub cooldowns: Vec<(String, String)>,
    pub last_turn: Option<SummaryFacts>,
    /// User turns in this session.
    pub turns: usize,
}

const LABEL_COLS: usize = 13;

fn status_row(label: &str, value: &str, style: Style) -> String {
    format!("  {}{value}", style.dim(&pad_to_width(label, LABEL_COLS)))
}

fn on_off(on: bool, style: Style) -> String {
    if on {
        style.green("on")
    } else {
        "off".to_string()
    }
}

/// `/status`: project, routing, pin, orchestration, ready providers with plan use,
/// cooldowns, the last turn and how many turns the session has had.
pub fn status_text(f: &StatusFacts, style: Style) -> String {
    let th = Theme::new(style);
    let mut rows = vec![style.bold("Session overview"), String::new(), status_row("Project", &one_line(&f.project), style)];
    let routing = if f.routing_on { on_off(true, style) } else { format!("off {}", style.dim("(Claude only)")) };
    rows.push(status_row("Routing", &routing, style));
    rows.push(status_row("Model", &one_line(&f.pin), style));
    rows.push(status_row("Orchestrate", &on_off(f.orchestrate, style), style));
    if f.ready.is_empty() {
        rows.push(status_row("Providers", &style.yellow("none ready: see /providers or `jev-code doctor`"), style));
    }
    for (i, p) in f.ready.iter().enumerate() {
        let mut v = format!("{} {}", style.green("●"), th.name(&p.provider));
        if let Some(plan) = &p.plan {
            v.push_str(&format!("  {}", style.dim(&one_line(plan))));
        }
        if let Some(used) = p.plan_used {
            let pct = (used.clamp(0.0, 1.0) * 100.0).round() as u32;
            let text = format!("plan {pct}% used");
            v.push_str(&format!("  {}", if pct >= 80 { style.yellow(&text) } else { style.dim(&text) }));
        }
        if let Some(credit) = &p.credit {
            v.push_str(&format!("  {}", style.dim(&one_line(credit))));
        }
        rows.push(status_row(if i == 0 { "Providers" } else { "" }, &v, style));
    }
    for (i, (p, left)) in f.cooldowns.iter().enumerate() {
        let v = format!("{} {}", th.name(p), style.yellow(&format!("for {left}")));
        rows.push(status_row(if i == 0 { "Cooling down" } else { "" }, &v, style));
    }
    let last = f.last_turn.as_ref().map_or_else(|| style.dim("none yet"), |t| turn_summary(t, style));
    rows.push(status_row("Last turn", &last, style));
    let turns = if f.turns == 0 { "no turns yet".to_string() } else { plural(f.turns, "turn", "turns") };
    rows.push(status_row("Session", &turns, style));
    rows.join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Ok,
    Info,
    Warn,
    Fail,
}

/// One finding of `jev-code doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorItem {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    /// Commands (or slash commands) that fix it.
    pub fix: Vec<String>,
}

fn severity_mark(s: Severity, style: Style) -> String {
    match s {
        Severity::Ok => style.green("✓"),
        Severity::Info => style.cyan("ⓘ"),
        Severity::Warn => style.yellow("⚠"),
        Severity::Fail => style.red("✗"),
    }
}

/// The doctor's findings in the order given (callers put problems first): a mark and
/// title per item, the detail dimmed and wrapped under it, and each fix on its own
/// indented line in cyan (`$ codex login`; a `/slash` command has no `$`). A blank line
/// separates items that have more than a title.
pub fn doctor_report(items: &[DoctorItem], style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let mut lines: Vec<String> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let title = truncate_to_width(&one_line(&it.title), width - 2);
        let title = if matches!(it.severity, Severity::Warn | Severity::Fail) { style.bold(&title) } else { title };
        lines.push(format!("{} {title}", severity_mark(it.severity, style)));
        let detail = it.detail.trim();
        if !detail.is_empty() {
            let clean = detail.lines().map(one_line).collect::<Vec<_>>().join("\n");
            lines.extend(wrap_to_width(&clean, width - 4).into_iter().map(|l| format!("    {}", style.dim(&l))));
        }
        for fix in &it.fix {
            let fix = one_line(fix);
            let cmd = if fix.starts_with('/') { fix } else { format!("$ {fix}") };
            lines.push(format!("    {}", style.cyan(&truncate_to_width(&cmd, width - 4))));
        }
        if (!detail.is_empty() || !it.fix.is_empty()) && i + 1 < items.len() {
            lines.push(String::new());
        }
    }
    lines.join("\n")
}

/// A closing line for the report: `All good`, or the counts of problems and warnings.
pub fn doctor_summary(items: &[DoctorItem], style: Style) -> String {
    let count = |s: Severity| items.iter().filter(|i| i.severity == s).count();
    let (fails, warns) = (count(Severity::Fail), count(Severity::Warn));
    match (fails, warns) {
        (0, 0) => style.green("All good"),
        (f, 0) => style.red(&plural(f, "problem", "problems")),
        (0, w) => style.yellow(&plural(w, "warning", "warnings")),
        (f, w) => format!("{} · {}", style.red(&plural(f, "problem", "problems")), style.yellow(&plural(w, "warning", "warnings"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::{strip_ansi, MIN_WIDTH};
    use crate::uifmt::testutil::*;

    #[test]
    fn help_lists_every_command_and_key() {
        let s = help_text(OFF, 100);
        no_esc(&s);
        for c in [
            "/help", "/status", "/providers [refresh]", "/doctor", "/model [target]", "/provider auto|off", "/orchestrate [on|off]",
            "/diff [--all|--turn N|--review|--split|--export F] [file]", "/undo", "/rewind [turn]", "/resume", "/new", "/quit",
        ] {
            assert!(s.contains(c), "{c}\n{s}");
        }
        for k in ["Ctrl-C", "stop the running turn", "Ctrl-D", "Alt-Enter", "↑ / ↓", "Tab"] {
            assert!(s.contains(k), "{k}");
        }
        let ls = lines(&s);
        let at = |h: &str| ls.iter().position(|l| *l == h).unwrap_or_else(|| panic!("{h}"));
        assert!(at("Model & routing") < at("Session") && at("Session") < at("Info") && at("Info") < at("Keys"));
        // each command sits in its group
        let line_of = |c: &str| ls.iter().position(|l| l.trim_start().starts_with(c)).unwrap();
        assert!(line_of("/model") > at("Model & routing") && line_of("/model") < at("Session"));
        assert!(line_of("/undo") > at("Session") && line_of("/undo") < at("Info"));
        assert!(line_of("/doctor") > at("Info") && line_of("/doctor") < at("Keys"));
        assert_eq!(strip_ansi(&help_text(ON, 100)), s);
        assert!(help_text(ON, 100).contains("\x1b[1mKeys\x1b[0m"));
    }

    #[test]
    fn help_fits_every_width_and_never_splits_a_word() {
        for width in [MIN_WIDTH, 50, 60, 80, 100, 200] {
            let s = help_text(OFF, width);
            for line in s.lines() {
                assert!(crate::uifmt::display_width(line) <= width, "{line:?} at {width}");
            }
            if width >= 50 {
                assert!(s.contains("Summarise the conversation"), "{s}");
            }
            assert!(!s.contains("summa\n"), "{s}");
        }
        let narrow = help_text(OFF, 50);
        assert!(narrow.contains("  /permissions [allow|remove|dir <x>]\n    Show or change"), "under the command when narrow: {narrow}");
    }

    #[test]
    fn help_columns_align() {
        let s = help_text(OFF, 100);
        let col = s.lines().find(|l| l.trim_start().starts_with("/new")).map(|l| l.find("Start").unwrap()).unwrap();
        let col2 = s.lines().find(|l| l.trim_start().starts_with("/model [target]")).map(|l| l.find("Pin").unwrap()).unwrap();
        assert_eq!(col, col2);
    }

    #[test]
    fn status_text_layout() {
        let f = StatusFacts {
            project: "/home/me/proj".into(),
            routing_on: true,
            pin: "auto".into(),
            orchestrate: false,
            ready: vec![
                ReadyProvider { provider: "claude".into(), plan: Some("max_5x".into()), plan_used: Some(0.32), credit: None },
                ReadyProvider { provider: "codex".into(), plan: None, plan_used: None, credit: None },
                ReadyProvider { provider: "deepseek".into(), plan: None, plan_used: None, credit: Some("$4.20 left".into()) },
            ],
            cooldowns: vec![("gemini".into(), "1h 5m".into())],
            last_turn: Some(SummaryFacts { ok: true, provider: "codex".into(), model: "g".into(), elapsed_ms: 2000, ..Default::default() }),
            turns: 4,
        };
        assert_eq!(
            status_text(&f, OFF),
            "Session overview\n\n  Project      /home/me/proj\n  Routing      on\n  Model        auto\n  Orchestrate  off\n  Providers    ● claude  max_5x  plan 32% used\n               ● codex\n               ● deepseek  $4.20 left\n  Cooling down gemini for 1h 5m\n  Last turn    ✓ codex · g · 2s\n  Session      4 turns"
        );
        let bare = StatusFacts { project: "/p".into(), pin: "codex".into(), turns: 1, ..Default::default() };
        assert_eq!(
            status_text(&bare, OFF),
            "Session overview\n\n  Project      /p\n  Routing      off (Claude only)\n  Model        codex\n  Orchestrate  off\n  Providers    none ready: see /providers or `jev-code doctor`\n  Last turn    none yet\n  Session      1 turn"
        );
        let fresh = StatusFacts { ..Default::default() };
        assert!(status_text(&fresh, OFF).ends_with("Session      no turns yet"));
        assert_eq!(strip_ansi(&status_text(&f, ON)), status_text(&f, OFF));
        let hot = StatusFacts { ready: vec![ReadyProvider { provider: "claude".into(), plan: None, plan_used: Some(0.9), credit: None }], ..Default::default() };
        assert!(status_text(&hot, ON).contains("\x1b[33mplan 90% used\x1b[0m"));
    }

    #[test]
    fn doctor_report_layout() {
        let items = vec![
            item(Severity::Fail, "codex is not logged in", "The probe could not see a login.", &["codex login", "/providers refresh"]),
            item(Severity::Warn, "No Jev key", "", &[]),
            item(Severity::Info, "Orchestration is off", "", &["/orchestrate on"]),
            item(Severity::Ok, "claude ready", "", &[]),
        ];
        assert_eq!(
            doctor_report(&items, OFF, 80),
            "✗ codex is not logged in\n    The probe could not see a login.\n    $ codex login\n    /providers refresh\n\n⚠ No Jev key\nⓘ Orchestration is off\n    /orchestrate on\n\n✓ claude ready"
        );
        assert_eq!(doctor_report(&[], OFF, 80), "");
        let styled = doctor_report(&items, ON, 80);
        assert!(styled.contains("\x1b[31m✗\x1b[0m") && styled.contains("\x1b[33m⚠\x1b[0m"));
        assert!(styled.contains("\x1b[36mⓘ\x1b[0m") && styled.contains("\x1b[32m✓\x1b[0m"));
        assert!(styled.contains("\x1b[36m$ codex login\x1b[0m"));
        assert!(styled.contains("\x1b[1mcodex is not logged in\x1b[0m"));
        assert_eq!(strip_ansi(&styled), doctor_report(&items, OFF, 80));
    }

    #[test]
    fn doctor_report_wraps_to_width() {
        let detail = "This is a long explanation of what went wrong with the setup, ".repeat(4);
        let items = vec![
            item(Severity::Fail, &"a very long title ".repeat(10), &detail, &["a-very-long-command --with --many --flags --that --do --not --fit --anywhere"]),
            item(Severity::Warn, "日本語のタイトル", "詳細な説明がここに入ります。とても長い説明です。", &[]),
        ];
        for w in [MIN_WIDTH, 30, 50, 80] {
            for style in [ON, OFF] {
                for l in doctor_report(&items, style, w).split('\n') {
                    assert!(display_width(l) <= w, "{l:?} at {w}");
                }
            }
        }
    }

    #[test]
    fn doctor_summaries() {
        let ok = item(Severity::Ok, "x", "", &[]);
        let warn = item(Severity::Warn, "x", "", &[]);
        let fail = item(Severity::Fail, "x", "", &[]);
        assert_eq!(doctor_summary(std::slice::from_ref(&ok), OFF), "All good");
        assert_eq!(doctor_summary(&[ok.clone(), warn.clone()], OFF), "1 warning");
        assert_eq!(doctor_summary(&[fail.clone(), fail.clone()], OFF), "2 problems");
        assert_eq!(doctor_summary(&[fail, warn.clone(), warn], OFF), "1 problem · 2 warnings");
        assert_eq!(doctor_summary(&[], ON), "\x1b[32mAll good\x1b[0m");
    }
}
