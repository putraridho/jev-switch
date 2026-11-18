//! What the live area looks like: the working row, queued messages, the input box (or an
//! approval card in its place), the menu and the footer, as rows of styled text and the
//! cursor's place. Pure: every screen is a snapshot test at any width, with and without
//! colour.
//!
//! Every row is cut to `width - 1` columns, so no row ever wraps (the screen erases the live
//! area by counting rows).

use std::time::Instant;

use super::app::App;
use super::menu::MENU_ROWS;
use crate::mode::Mode;
use crate::uifmt::{display_width, truncate_to_width, wrap_to_width, Spinner, Style};

/// The most rows of typed text the box shows; longer text scrolls with the cursor.
pub const INPUT_ROWS: usize = 8;
/// The most rows of a tool's input the approval card shows at once.
pub const CARD_INPUT_ROWS: usize = 12;

/// One drawn live area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub rows: Vec<String>,
    /// Row and column of the text cursor; `None` hides it.
    pub cursor: Option<(usize, usize)>,
}

/// The roles colour plays on this screen, in one place. Nothing here is colour alone: each
/// role also has a glyph or a word.
struct Theme(Style);

impl Theme {
    fn provider_theme(style: Style) -> crate::uifmt::Theme {
        crate::uifmt::Theme::new(style)
    }
    fn accent(&self, s: &str) -> String {
        self.0.cyan(s)
    }
    fn muted(&self, s: &str) -> String {
        self.0.dim(s)
    }
    fn strong(&self, s: &str) -> String {
        self.0.bold(s)
    }
    fn warning(&self, s: &str) -> String {
        self.0.yellow(s)
    }
    fn border(&self, s: &str) -> String {
        self.0.dim(s)
    }
    fn selected(&self, s: &str) -> String {
        self.0.bold(&self.0.cyan(s))
    }
    /// The mode's name, coloured by how much it lets the agent do.
    fn mode(&self, m: Mode) -> String {
        let text = format!("{} mode", m.name());
        match m {
            Mode::Auto => self.0.yellow(&text),
            Mode::Plan => self.0.cyan(&text),
            Mode::Edit => self.0.green(&text),
            Mode::Ask => text,
        }
    }
}

/// `left` and `right` on one row of `width` columns, `right` flush right when both fit.
fn spread(left: &str, right: &str, width: usize) -> String {
    let (lw, rw) = (display_width(left), display_width(right));
    if right.is_empty() || lw + rw + 2 > width {
        return truncate_to_width(left, width);
    }
    format!("{left}{}{right}", " ".repeat(width - lw - rw))
}

/// A box border row: `╭─ title ─────╮` (or the bottom one).
fn border(left: char, right: char, title: &str, width: usize, th: &Theme) -> String {
    let inner = width.saturating_sub(2);
    if title.is_empty() {
        return th.border(&format!("{left}{}{right}", "─".repeat(inner)));
    }
    let shown = truncate_to_width(title, inner.saturating_sub(4));
    let fill = inner.saturating_sub(display_width(&shown) + 3);
    format!("{}{}{}", th.border(&format!("{left}─ ")), th.strong(&shown), th.border(&format!(" {}{right}", "─".repeat(fill))))
}

/// A row inside a box: `│ content   │`, content cut or padded to fit.
fn boxed(content: &str, width: usize, th: &Theme) -> String {
    let inner = width.saturating_sub(4);
    let shown = truncate_to_width(content, inner);
    let pad = inner.saturating_sub(display_width(&shown));
    format!("{} {shown}{} {}", th.border("│"), " ".repeat(pad), th.border("│"))
}

/// What the model is doing, said as an action: `Bash cargo test` → `Running cargo test`.
pub fn doing(activity: Option<&str>) -> String {
    let Some(a) = activity.map(str::trim).filter(|a| !a.is_empty()) else { return "Thinking".to_string() };
    let (tool, rest) = a.split_once(' ').unwrap_or((a, ""));
    let verb = match tool.to_ascii_lowercase().as_str() {
        "bash" | "shell" => "Running",
        "read" => "Reading",
        "edit" | "multiedit" => "Editing",
        "write" => "Writing",
        "grep" | "glob" | "ls" => "Searching",
        "webfetch" | "websearch" => "Looking up",
        "task" | "agent" => "Delegating",
        _ => return a.to_string(),
    };
    if rest.is_empty() { verb.to_string() } else { format!("{verb} {rest}") }
}

/// The working row: `⠹ Running cargo test · 12s · claude · sonnet-5-5`. What is happening
/// comes first, then how long, then who is doing it. Without colour the spinner is a still
/// mark (no colour means no animation either).
pub fn working_line(work: &super::app::Working, elapsed: std::time::Duration, width: usize, style: Style) -> String {
    let spinner = if style.enabled { Spinner::frame_for(elapsed) } else { "•" };
    let secs = elapsed.as_secs();
    let time = if secs >= 60 { format!("{}m{:02}s", secs / 60, secs % 60) } else { format!("{secs}s") };
    let what = doing(work.activity.as_deref());
    let who = if work.label == "thinking" { String::new() } else { format!(" · {}", work.label) };
    truncate_to_width(&format!("{} {} {}", style.cyan(spinner), style.bold(&what), style.dim(&format!("· {time}{who}"))), width)
}

/// Draw the live area for `width` terminal columns at time `now`.
pub fn render(app: &App, width: usize, now: Instant, style: Style) -> Frame {
    let th = Theme(style);
    // One column short of the terminal, so a full row never wraps.
    let w = width.saturating_sub(1).max(20);
    let mut rows: Vec<String> = Vec::new();
    let mut cursor = None;

    // The working row steps aside while a question waits: one thing to look at.
    if let Some(work) = app.working.as_ref().filter(|_| app.approval.is_none()) {
        let hint = "esc to stop";
        let budget = w.saturating_sub(display_width(hint) + 2);
        let elapsed = now.duration_since(work.started);
        let line = working_line(work, elapsed, budget, style);
        rows.push(spread(&line, &th.muted(hint), w));
    }
    for (i, queued) in app.queue.iter().take(3).enumerate() {
        let first = queued.lines().next().unwrap_or_default();
        let more = if queued.lines().count() > 1 { " …" } else { "" };
        let tag = if i == 0 && app.working.is_none() { "(next)" } else { "(queued)" };
        rows.push(truncate_to_width(&format!("  {} {first}{more} {}", th.accent("↳"), th.muted(tag)), w));
    }
    if app.queue.len() > 3 {
        rows.push(th.muted(&format!("  … and {} more queued", app.queue.len() - 3)));
    }

    if let Some(c) = &app.confirm {
        rows.push(border('╭', '╮', &c.question, w, &th));
        for (i, label) in ["Yes", "No"].iter().enumerate() {
            let line = if i == c.selected { th.selected(&format!("› {}. {label}", i + 1)) } else { format!("  {}. {label}", i + 1) };
            rows.push(boxed(&line, w, &th));
        }
        rows.push(border('╰', '╯', "", w, &th));
        rows.push(th.muted("  y yes · n or esc no · ↑↓ and enter"));
    } else if let Some(a) = &app.approval {
        rows.extend(approval_card(a, w, &th));
    } else {
        let (box_rows, at) = input_box(app, w, &th);
        let top = rows.len();
        rows.extend(box_rows);
        cursor = Some((top + at.0, at.1));
        if let Some(m) = &app.menu {
            rows.extend(menu_rows(m, w, &th));
        }
        rows.push(router_row(app, w, &th));
        rows.push(footer(app, w, now, &th));
    }
    Frame { rows, cursor }
}

/// The input box and where the cursor is in its rows.
fn input_box(app: &App, w: usize, th: &Theme) -> (Vec<String>, (usize, usize)) {
    // Inside the borders: `│ › ` before the text, ` │` after.
    let text_w = w.saturating_sub(6).max(1);
    let mut rows = vec![border('╭', '╮', "", w, th)];
    if app.editor.is_empty() {
        let placeholder = if app.working.is_some() { "Type to queue a message for after this turn" } else { "Ask anything · / for commands · @ for files" };
        rows.push(boxed(&format!("{} {}", th.accent("›"), th.muted(placeholder)), w, th));
        rows.push(border('╰', '╯', "", w, th));
        return (rows, (1, 4));
    }
    let (lines, (cr, cc)) = app.editor.layout(text_w);
    let first = cr.saturating_sub(INPUT_ROWS - 1);
    for (i, line) in lines.iter().enumerate().skip(first).take(INPUT_ROWS) {
        let lead = if i == 0 { th.accent("›") } else { " ".to_string() };
        rows.push(boxed(&format!("{lead} {line}"), w, th));
    }
    let hidden = lines.len().saturating_sub(first + INPUT_ROWS);
    let bottom = if first > 0 || hidden > 0 { format!("{} lines", lines.len()) } else { String::new() };
    rows.push(border('╰', '╯', &bottom, w, th));
    (rows, (1 + cr - first, 4 + cc))
}

fn menu_rows(m: &super::menu::Menu, w: usize, th: &Theme) -> Vec<String> {
    let first = m.first_shown();
    // The label column fits the rows on screen, not the longest label anywhere.
    let label_w = m.items.iter().skip(first).take(MENU_ROWS).map(|i| display_width(&i.label)).max().unwrap_or(0).min(w / 2).max(8);
    let mut rows = Vec::new();
    for (i, item) in m.items.iter().enumerate().skip(first).take(MENU_ROWS) {
        let label = truncate_to_width(&item.label, label_w);
        let pad = " ".repeat(label_w.saturating_sub(display_width(&label)) + 2);
        let detail = truncate_to_width(&item.detail, w.saturating_sub(label_w + 6));
        let row = if i == m.selected {
            format!("  {} {}{pad}{}", th.accent("›"), th.selected(&label), detail)
        } else {
            format!("    {label}{pad}{}", th.muted(&detail))
        };
        rows.push(truncate_to_width(&row, w));
    }
    let more = m.items.len().saturating_sub(first + MENU_ROWS);
    let position = if m.items.len() > MENU_ROWS { format!("{}/{} · ", m.selected + 1, m.items.len()) } else { String::new() };
    let keys = if more > 0 || first > 0 { "↑↓ more · tab complete · enter run · esc close" } else { "↑↓ select · tab complete · enter run · esc close" };
    rows.push(truncate_to_width(&th.muted(&format!("    {position}{keys}")), w));
    rows
}

fn approval_card(a: &super::app::Approval, w: usize, th: &Theme) -> Vec<String> {
    let inner = w.saturating_sub(4).max(1);
    let mut rows = vec![border('╭', '╮', &format!("Allow {}?", a.tool), w, th)];
    let mut body: Vec<String> = Vec::new();
    for line in &a.lines {
        body.extend(wrap_to_width(line, inner));
    }
    let first = a.scroll.min(body.len().saturating_sub(1));
    for line in body.iter().skip(first).take(CARD_INPUT_ROWS) {
        rows.push(boxed(line, w, th));
    }
    let below = body.len().saturating_sub(first + CARD_INPUT_ROWS);
    if first > 0 || below > 0 {
        rows.push(boxed(&th.muted(&format!("… {} more lines · pgup/pgdn to scroll", first + below)), w, th));
    }
    for note in &a.notes {
        rows.push(boxed(&th.muted(note), w, th));
    }
    rows.push(boxed("", w, th));
    for (i, (_, label)) in a.choices().iter().enumerate() {
        let text = format!("{}. {label}", i + 1);
        let line = if i == a.selected { format!("{} {}", th.accent("›"), th.selected(&text)) } else { format!("  {text}") };
        rows.push(boxed(&line, w, th));
    }
    rows.push(border('╰', '╯', "", w, th));
    if let Some(note) = &a.note {
        for line in wrap_to_width(note, w.saturating_sub(2)) {
            rows.push(format!("  {}", th.warning(&line)));
        }
    }
    let n = a.choices().len();
    rows.push(truncate_to_width(&th.muted(&format!("  1–{n} or ↑↓ enter · esc no · ctrl-c stops the turn")), w));
    rows
}

/// The router row: how turns are routed and what each subscription has left:
/// `  ⇄ auto · claude 32% · codex 71% · gemini cooling 1h20m · orchestrate on`. A plan window
/// at 80% or more is yellow (the router will move off it soon).
fn router_row(app: &App, w: usize, th: &Theme) -> String {
    let style = th.0;
    let names = Theme::provider_theme(style);
    let mut parts = vec![format!("{} {}", th.accent("⇄"), app.route)];
    for p in &app.providers {
        let name = names.name(&p.name);
        let chip = match (&p.cooling, p.used) {
            (Some(left), _) => format!("{name} {}", th.muted(&format!("cooling {left}"))),
            (None, Some(u)) if u >= 80 => format!("{name} {}", th.warning(&format!("{u}%"))),
            (None, Some(u)) => format!("{name} {}", th.muted(&format!("{u}%"))),
            (None, None) => name,
        };
        parts.push(chip);
    }
    if app.providers.is_empty() {
        parts.push(th.warning("no provider ready · /doctor"));
    }
    if app.orchestrate {
        parts.push(th.muted("orchestrate on"));
    }
    truncate_to_width(&format!("  {}", parts.join(&th.muted(" · "))), w)
}

fn footer(app: &App, w: usize, now: Instant, th: &Theme) -> String {
    let mut left = format!("  {} {}", th.mode(app.mode), th.muted("(shift-tab)"));
    if let Some(git) = &app.git {
        left.push_str(&th.muted(&format!(" · ⎇ {git}")));
    }
    if let Some(name) = &app.session_name {
        left.push_str(&th.muted(&format!(" · \u{201c}{name}\u{201d}")));
    }
    // The context window only shows once it starts to matter.
    if let Some(pct) = app.context.filter(|p| *p >= 50) {
        let text = format!(" · context {pct}%");
        left.push_str(&if pct >= 80 { th.warning(&format!("{text} · /compact")) } else { th.muted(&text) });
    }
    let right = match &app.notice {
        Some((text, at)) if now.duration_since(*at) < super::app::NOTICE_FOR => th.warning(text),
        _ if app.working.is_some() => th.muted("enter queue · esc stop"),
        _ => th.muted("enter send · shift-enter newline"),
    };
    spread(&left, &right, w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::front::{ApprovalAsk, FrontMsg};
    use crate::tui::history::History;
    use crate::uifmt::strip_ansi;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::Path;

    const OFF: Style = Style { enabled: false, terminal: false };
    const ON: Style = Style { enabled: true, terminal: true };

    fn app() -> App {
        App::new(Path::new("/nonexistent").into(), History::default(), crate::mode::Mode::Edit, "auto".into(), Vec::new())
    }

    fn typed(a: &mut App, text: &str) {
        for c in text.chars() {
            a.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), Instant::now());
        }
    }

    fn screen(a: &App, width: usize) -> String {
        render(a, width, Instant::now(), OFF).rows.join("\n")
    }

    #[test]
    fn the_idle_box_has_a_placeholder_and_a_footer_with_the_mode() {
        let a = app();
        let f = render(&a, 70, Instant::now(), OFF);
        let rule = "─".repeat(67);
        assert_eq!(
            &f.rows[..3],
            &[format!("╭{rule}╮"), format!("│ › Ask anything · / for commands · @ for files{}│", " ".repeat(21)), format!("╰{rule}╯")]
        );
        assert!(f.rows[3].starts_with("  ⇄ auto"), "the router row: {}", f.rows[3]);
        assert!(f.rows[4].starts_with("  edit mode (shift-tab)") && f.rows[4].ends_with("enter send · shift-enter newline"), "{}", f.rows[4]);
        assert_eq!(f.cursor, Some((1, 4)));
    }

    #[test]
    fn typing_a_slash_shows_the_command_menu_with_descriptions() {
        let mut a = app();
        typed(&mut a, "/");
        let s = screen(&a, 80);
        assert!(s.contains("│ › /"), "{s}");
        assert!(s.contains("  › /model [target]"), "the first command is selected: {s}");
        assert!(s.contains("Pin a model or provider"), "{s}");
        assert!(s.contains("1/37 · ↑↓ more · tab complete · enter run · esc close"), "{s}");
        assert_eq!(s.lines().filter(|l| l.starts_with("    /") || l.starts_with("  › /")).count(), MENU_ROWS);
    }

    #[test]
    fn a_long_message_wraps_inside_the_box_and_the_cursor_follows() {
        let mut a = app();
        typed(&mut a, &"word ".repeat(20));
        let f = render(&a, 40, Instant::now(), OFF);
        let inside: Vec<_> = f.rows.iter().filter(|r| r.starts_with('│')).collect();
        assert!(inside.len() >= 3, "{:?}", f.rows);
        let (row, col) = f.cursor.unwrap_or_default();
        assert!(row >= 3 && col >= 4, "{:?}", f.cursor);
    }

    #[test]
    fn a_running_turn_shows_the_working_row_queue_and_stop_hint() {
        let mut a = app();
        a.on_front(FrontMsg::TurnStarted);
        a.on_front(FrontMsg::Label("claude · sonnet-5-5".into()));
        a.on_front(FrontMsg::Activity("Bash cargo test".into()));
        a.queue.push_back("then run clippy".into());
        let s = screen(&a, 80);
        let first = s.lines().next().unwrap_or_default();
        assert!(first.starts_with("• Running cargo test · 0s · claude · sonnet-5-5") && first.ends_with("esc to stop"), "a still mark without colour: {first}");
        assert!(s.contains("↳ then run clippy (queued)"), "{s}");
        assert!(s.contains("Type to queue a message for after this turn"), "{s}");
        assert!(s.contains("enter queue · esc stop"), "{s}");
    }

    #[test]
    fn the_working_row_says_what_is_happening_as_an_action() {
        assert_eq!(doing(None), "Thinking");
        assert_eq!(doing(Some("Bash cargo test")), "Running cargo test");
        assert_eq!(doing(Some("Read src/a.rs")), "Reading src/a.rs");
        assert_eq!(doing(Some("Grep TODO")), "Searching TODO");
        assert_eq!(doing(Some("stopping...")), "stopping...");
    }

    #[test]
    fn an_approval_replaces_the_box_with_numbered_choices() {
        let mut a = app();
        let (reply, _rx) = tokio::sync::oneshot::channel();
        a.on_front(FrontMsg::Approval(Box::new(ApprovalAsk {
            tool: "Bash".into(),
            notes: vec!["runs in the background".into()],
            lines: vec!["cargo test --quiet".into()],
            always: Some("Bash(cargo test:*)".into()),
            note: None,
            reply,
        })));
        let f = render(&a, 70, Instant::now(), OFF);
        let s = f.rows.join("\n");
        assert!(s.starts_with("╭─ Allow Bash? ─"), "{s}");
        assert!(s.contains("│ cargo test --quiet"), "{s}");
        assert!(s.contains("│ › 1. Yes "), "{s}");
        assert!(s.contains("│   2. Yes, and don't ask again (Bash(cargo test:*))"), "{s}");
        assert!(s.contains("│   3. No, and tell the model what to do instead"), "{s}");
        assert!(s.contains("1–3 or ↑↓ enter · esc no"), "{s}");
        assert_eq!(f.cursor, None, "no text cursor on a card");
    }

    #[test]
    fn every_row_fits_one_column_short_of_the_terminal_at_every_width_and_style() {
        let mut a = app();
        typed(&mut a, "/");
        a.on_front(FrontMsg::TurnStarted);
        a.queue.push_back("日本語のメッセージ ".repeat(10));
        for width in [1, 10, 20, 21, 40, 79, 80, 200] {
            for style in [ON, OFF] {
                let f = render(&a, width, Instant::now(), style);
                for row in &f.rows {
                    assert!(display_width(row) < width.max(21), "{:?} at {width}", strip_ansi(row));
                    assert!(!row.contains('\n'));
                }
                if !style.enabled {
                    assert!(f.rows.iter().all(|r| !r.contains('\x1b')));
                }
            }
        }
    }

    #[test]
    fn the_router_row_shows_each_subscription_and_what_it_has_left() {
        use crate::tui::app::ProviderChip;
        let mut a = app();
        a.providers = vec![
            ProviderChip { name: "claude".into(), used: Some(32), cooling: None },
            ProviderChip { name: "codex".into(), used: Some(85), cooling: None },
            ProviderChip { name: "gemini".into(), used: None, cooling: Some("1h20m".into()) },
        ];
        a.orchestrate = true;
        let s = screen(&a, 120);
        assert!(s.contains("  ⇄ auto · claude 32% · codex 85% · gemini cooling 1h20m · orchestrate on"), "{s}");
        a.providers.clear();
        assert!(screen(&a, 120).contains("⇄ auto · no provider ready · /doctor"));
    }

    #[test]
    fn the_footer_shows_the_branch_and_a_filling_context_window() {
        let mut a = app();
        a.git = Some("main ±3".into());
        a.context = Some(30);
        let s = screen(&a, 120);
        assert!(s.contains("edit mode (shift-tab) · ⎇ main ±3  "), "{s}");
        assert!(!s.contains("context"), "30% does not matter yet: {s}");
        a.context = Some(85);
        assert!(screen(&a, 120).contains("· context 85% · /compact"));
    }

    #[test]
    fn a_notice_shows_in_the_footer_and_the_mode_is_named_not_only_coloured() {
        let mut a = app();
        a.notice = Some(("Press Ctrl-C again to exit".into(), Instant::now()));
        let s = screen(&a, 80);
        assert!(s.lines().last().is_some_and(|l| l.contains("edit mode") && l.ends_with("Press Ctrl-C again to exit")), "{s}");
    }
}
