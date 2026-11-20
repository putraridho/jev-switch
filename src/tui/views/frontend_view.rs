//! What the screen looks like around the conversation: the header row (where you are, what the
//! router has), the working row, queued messages, the `/` menu, the input box (or an approval
//! card in its place) and the key hints under it, as rows of styled text and the cursor's
//! place. Pure: every screen is a snapshot test at any width, with and without colour.
//!
//! The look follows one rule: only the input box is boxed. Everything else is a row of text,
//! set apart by space, weight and a shaded band, so the conversation keeps the room.
//!
//! Every row is cut to `width - 1` columns, so no row ever wraps (the screen erases the live
//! area by counting rows).

use std::time::Instant;

use crate::tui::app::screen_state::App;
use crate::tui::input::menu::MENU_ROWS;
use crate::approval::domain::mode::Mode;
use crate::tui::render::{answer_panel, answer_text, display_width, truncate_to_width, wrap_to_width, Spinner, Style};

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
    fn provider_theme(style: Style) -> crate::tui::render::Theme {
        crate::tui::render::Theme::new(style)
    }
    fn accent(&self, s: &str) -> String {
        self.0.accent(s)
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
    /// The input box's edge: a control's boundary, so it stays visible.
    fn field(&self, s: &str) -> String {
        self.0.field(s)
    }
    /// A raised surface (the menu's selected row, an approval card): a step off the background.
    fn raised(&self, s: &str) -> String {
        self.0.band(s)
    }
    fn selected(&self, s: &str) -> String {
        self.0.bold(s)
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
    /// Key hints on one row of `width`, `  Enter:send │ Shift+Tab:mode`: the key bright, what it
    /// does dim. The ones that do not fit are left off whole, never cut in half.
    fn keys(&self, pairs: &[(&str, &str)], width: usize) -> String {
        let sep = self.muted(" │ ");
        let mut row = String::from(" ");
        for (i, (k, does)) in pairs.iter().enumerate() {
            let hint = format!("{}{}", self.strong(k), self.muted(&format!(":{does}")));
            let next = if i == 0 { format!("{row} {hint}") } else { format!("{row}{sep}{hint}") };
            if display_width(&next) > width {
                break;
            }
            row = next;
        }
        row
    }
}

/// `left` and `right` on one row of `width` columns, `right` flush right. When both do not
/// fit, `left` is cut to make room, down to a few columns; past that `right` gives way.
fn spread(left: &str, right: &str, width: usize) -> String {
    let (lw, rw) = (display_width(left), display_width(right));
    if right.is_empty() {
        return truncate_to_width(left, width);
    }
    if lw + rw + 2 > width {
        let room = width.saturating_sub(rw + 2);
        if room < 12 {
            return truncate_to_width(left, width);
        }
        let cut = truncate_to_width(left, room);
        return format!("{cut}{}{right}", " ".repeat(width - display_width(&cut) - rw));
    }
    format!("{left}{}{right}", " ".repeat(width - lw - rw))
}

/// A box border row: `╭─ title ─────╮` (or the bottom one), its lines in `edge`.
fn frame(left: char, right: char, title: &str, width: usize, th: &Theme, edge: impl Fn(&str) -> String) -> String {
    let inner = width.saturating_sub(2);
    if title.is_empty() {
        return edge(&format!("{left}{}{right}", "─".repeat(inner)));
    }
    let shown = truncate_to_width(title, inner.saturating_sub(4));
    let fill = inner.saturating_sub(display_width(&shown) + 3);
    format!("{}{}{}", edge(&format!("{left}─ ")), th.strong(&shown), edge(&format!(" {}{right}", "─".repeat(fill))))
}

/// The input box's bottom edge: a hint set in on the left (`→ to use`, `12 lines`) and the
/// route and mode on the right, `╰─ → to use ──────── auto · edit mode ─╯`.
fn bottom_edge(hint: &str, label: &str, width: usize, th: &Theme) -> String {
    let inner = width.saturating_sub(2);
    let label_w = display_width(label);
    let left = if hint.is_empty() { String::new() } else { format!("{}{} ", th.field("─ "), th.muted(hint)) };
    let left_w = display_width(&crate::tui::render::strip_ansi(&left));
    // Too narrow for both: the label goes, the hint stays.
    if left_w + label_w + 5 > inner {
        let fill = inner.saturating_sub(left_w);
        return format!("{}{left}{}", th.field("╰"), th.field(&format!("{}╯", "─".repeat(fill))));
    }
    let fill = inner - left_w - label_w - 3;
    format!("{}{left}{}{label}{}", th.field("╰"), th.field(&format!("{} ", "─".repeat(fill))), th.field(" ─╯"))
}

/// A row inside a box: `│ content   │`, content cut or padded to fit, its sides in `edge`.
fn boxed_in(content: &str, width: usize, edge: impl Fn(&str) -> String) -> String {
    let inner = width.saturating_sub(4);
    let shown = truncate_to_width(content, inner);
    let pad = inner.saturating_sub(display_width(&shown));
    format!("{} {shown}{} {}", edge("│"), " ".repeat(pad), edge("│"))
}

/// A row on a raised surface: shaded the whole `width`, an accent bar `▌` on its left edge.
fn raised_row(content: &str, width: usize, th: &Theme) -> String {
    let inner = width.saturating_sub(4);
    let shown = truncate_to_width(content, inner);
    let pad = inner.saturating_sub(display_width(&shown));
    th.raised(&format!("{} {shown}{}  ", th.accent("▌"), " ".repeat(pad)))
}

/// What the model is doing, said as an action: `Bash cargo test` → `Running cargo test`.
pub fn doing(activity: Option<&str>) -> String {
    let (verb, rest) = doing_parts(activity);
    if rest.is_empty() { verb } else { format!("{verb} {rest}") }
}

/// [`doing`] in its two parts, the verb and what it acts on, so they can be weighted apart.
fn doing_parts(activity: Option<&str>) -> (String, String) {
    let Some(a) = activity.map(str::trim).filter(|a| !a.is_empty()) else { return ("Thinking".to_string(), String::new()) };
    let (tool, rest) = a.split_once(' ').unwrap_or((a, ""));
    let verb = match tool.to_ascii_lowercase().as_str() {
        "bash" | "shell" => "Running",
        "read" => "Reading",
        "edit" | "multiedit" => "Editing",
        "write" => "Writing",
        "grep" | "glob" | "ls" => "Searching",
        "webfetch" | "websearch" => "Looking up",
        "task" | "agent" => "Delegating",
        _ => return (a.to_string(), String::new()),
    };
    (verb.to_string(), rest.to_string())
}

/// The working row: `⠹ Running cargo test 12s`, the verb dim and what it acts on in the accent,
/// then how long. Without colour the spinner is a still mark (no colour means no animation).
pub fn working_line(work: &crate::tui::app::screen_state::Working, elapsed: std::time::Duration, width: usize, style: Style) -> String {
    let spinner = if style.enabled { Spinner::frame_for(elapsed) } else { "•" };
    let secs = elapsed.as_secs();
    let time = if secs >= 60 { format!("{}m{:02}s", secs / 60, secs % 60) } else { format!("{secs}s") };
    let (verb, rest) = doing_parts(work.activity.as_deref());
    let what = if rest.is_empty() { style.bold(&verb) } else { format!("{} {}", style.dim(&verb), style.accent(&rest)) };
    truncate_to_width(&format!("{} {what} {}", style.accent(spinner), style.dim(&time)), width)
}

/// The most workers listed by name above the box; the rest are counted.
const WORKER_ROWS: usize = 4;

/// What the planner's workers are doing, above the box: a heading row,
/// `▾ Workers · 1 running · 1 to review`, then a row each (`▶ w1  gemini · argon  running 0:42`,
/// `✓ w2  codex  ready to review`). Nothing when no worker is open.
fn workers_rows(app: &App, w: usize, style: Style) -> Vec<String> {
    if app.workers.is_empty() {
        return Vec::new();
    }
    let th = Theme(style);
    let names = Theme::provider_theme(style);
    let running = app.workers.iter().filter(|x| x.running).count();
    let waiting = app.workers.len() - running;
    let mut title = String::from("Workers");
    if running > 0 {
        title.push_str(&format!(" · {running} running"));
    }
    if waiting > 0 {
        title.push_str(&format!(" · {waiting} to review"));
    }
    let mut rows = vec![truncate_to_width(&format!("  {} {}", th.muted("▾"), th.muted(&th.strong(&title))), w)];
    for wk in app.workers.iter().take(WORKER_ROWS) {
        let (glyph, state) = if wk.running {
            (style.accent("▶"), style.dim(&format!("running {}", clock(wk.age_ms))))
        } else {
            (style.yellow("✓"), style.yellow("ready to review"))
        };
        let who = if wk.model.is_empty() { names.name(&wk.provider) } else { format!("{} {} {}", names.name(&wk.provider), style.dim("·"), wk.model) };
        let calls = if wk.running && wk.tools > 0 { style.dim(&format!(" · {} tool calls", wk.tools)) } else { String::new() };
        rows.push(truncate_to_width(&format!("  {glyph} {}  {who}   {state}{calls}", style.bold(&wk.id)), w));
        // What a running worker is doing right now, under it.
        if let Some(doing) = wk.activity.as_deref().filter(|_| wk.running) {
            rows.push(truncate_to_width(&format!("      {}", th.muted(&format!("⎿ {doing}"))), w));
        }
    }
    if app.workers.len() > WORKER_ROWS {
        rows.push(truncate_to_width(&format!("    {}", style.dim(&format!("… and {} more", app.workers.len() - WORKER_ROWS))), w));
    }
    rows
}

/// `0:42`, `12:05`: a worker's age.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Where the live area is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Under the conversation in the terminal's own scrollback: the header rides on top of it.
    Inline,
    /// Full screen: the header is pinned to the top row instead.
    Full,
}

/// Draw the live area for `width` terminal columns at time `now`.
pub fn render(app: &App, width: usize, now: Instant, style: Style) -> Frame {
    live(app, width, now, style, Place::Inline)
}

fn live(app: &App, width: usize, now: Instant, style: Style, place: Place) -> Frame {
    let th = Theme(style);
    // One column short of the terminal, so a full row never wraps.
    let w = width.saturating_sub(1).max(20);
    let mut rows: Vec<String> = Vec::new();
    let mut cursor = None;
    if place == Place::Inline {
        rows.push(header_row(app, w, &th));
    }

    // The working row steps aside while a question waits: one thing to look at.
    if let Some(work) = app.working.as_ref().filter(|_| app.approval.is_none()) {
        let hint = "esc to stop";
        let elapsed = now.duration_since(work.started);
        // Who is working goes first when the row is short: what runs and how to stop it stay.
        let who = if work.label == "thinking" { String::new() } else { format!("{} · ", work.label) };
        let full = display_width(&working_line(work, elapsed, usize::MAX, style)) + 2;
        let right = if full + display_width(&who) + display_width(hint) + 2 <= w { format!("{who}{hint}") } else { hint.to_string() };
        let line = working_line(work, elapsed, w.saturating_sub(display_width(&right) + 4), style);
        rows.push(spread(&format!("  {line}"), &th.muted(&right), w));
    }
    rows.extend(workers_rows(app, w, style));
    for (i, queued) in app.queue.iter().take(3).enumerate() {
        let first = queued.lines().next().unwrap_or_default();
        let more = if queued.lines().count() > 1 { " …" } else { "" };
        rows.push(truncate_to_width(&format!("  {} {first}{more}", th.muted(&format!("#{}", i + 1))), w));
    }
    if app.queue.len() > 3 {
        rows.push(th.muted(&format!("  … and {} more queued", app.queue.len() - 3)));
    }

    if let Some(c) = &app.confirm {
        rows.push(raised_row(&th.strong(&c.question), w, &th));
        rows.push(raised_row("", w, &th));
        for (i, label) in ["Yes", "No"].iter().enumerate() {
            rows.push(raised_row(&choice(i, label, i == c.selected, &th), w, &th));
        }
        rows.push(th.keys(&[("y", "yes"), ("n/Esc", "no"), ("↑↓", "move"), ("Enter", "choose")], w));
    } else if let Some(a) = &app.approval {
        rows.extend(approval_card(a, w, &th));
    } else {
        if let Some(m) = &app.menu {
            rows.extend(menu_rows(m, w, &th));
        }
        let (box_rows, at) = input_box(app, w, &th);
        cursor = Some((rows.len() + at.0, at.1));
        rows.extend(box_rows);
        rows.push(footer(app, w, now, &th));
    }
    Frame { rows, cursor }
}

/// One numbered choice on a card: `● 1. Yes` when selected, `○ 2. No` otherwise.
fn choice(i: usize, label: &str, selected: bool, th: &Theme) -> String {
    let text = format!("{}. {label}", i + 1);
    if selected {
        format!("{} {}", th.accent("●"), th.selected(&text))
    } else {
        format!("{} {}", th.muted("○"), th.muted(&text))
    }
}

/// The whole terminal, `cols` by `height`: the header pinned on top, the chat under it, and
/// the input box with its key hints at the bottom. Returns the frame (exactly `height` rows)
/// and how many rows the chat pane has.
pub fn render_full(app: &App, cols: usize, height: usize, now: Instant, style: Style) -> (Frame, usize) {
    let w = cols.saturating_sub(1).max(20);
    let mut live = live(app, cols, now, style, Place::Full);
    // A terminal too short for the live area keeps its bottom rows (the box and the keys).
    if live.rows.len() > height {
        let cut = live.rows.len() - height;
        live.rows.drain(..cut);
        live.cursor = live.cursor.and_then(|(r, c)| r.checked_sub(cut).map(|r| (r, c)));
    }
    // The header takes the top row once there is a row to spare for it.
    let header = usize::from(height > live.rows.len());
    let chat_h = height - live.rows.len() - header;
    let mut rows = Vec::with_capacity(height);
    if header == 1 {
        rows.push(header_row(app, w, &Theme(style)));
    }
    rows.extend(chat_rows(app, w, chat_h, style));
    let top = rows.len();
    rows.extend(live.rows);
    let cursor = live.cursor.map(|(r, c)| (r + top, c));
    (Frame { rows, cursor }, chat_h)
}

/// The conversation rows for the chat pane, ending with the answer as it is being written.
/// The preview is shown only while following the end; it never enters the transcript, and
/// the finished answer replaces it when the turn commits.
fn chat_rows(app: &App, width: usize, height: usize, style: Style) -> Vec<String> {
    let preview = app.live_answer.as_deref().filter(|t| !t.trim().is_empty()).filter(|_| app.transcript.scrolled() == 0);
    let Some(text) = preview else {
        return app.transcript.window(height).iter().map(|l| truncate_to_width(l, width)).collect();
    };
    let laid = answer_panel(&answer_text(text, style, width.saturating_sub(4)), style);
    let mut rows: Vec<String> = laid.split('\n').map(|l| truncate_to_width(l, width)).collect();
    // A long answer keeps its newest rows, like the conversation below the fold.
    if rows.len() > height {
        rows.drain(..rows.len() - height);
    }
    let above = height - rows.len();
    let mut all: Vec<String> = app.transcript.window(above).iter().map(|l| truncate_to_width(l, width)).collect();
    all.extend(rows);
    all
}

/// The header row: where you are on the left, `main ±3 ~/Playground/jev-switch “name”`, the
/// branch dim and the folder bright; what the router has on the right,
/// `⇄ auto · claude 32% · codex 71%  │  context 12%`.
fn header_row(app: &App, w: usize, th: &Theme) -> String {
    let mut right = router_text(app, th);
    if let Some(pct) = app.context {
        let text = format!("context {pct}%");
        let shown = if pct >= 80 { th.warning(&format!("{text} · /compact")) } else { th.muted(&text) };
        right.push_str(&format!("{}{shown}", th.muted("  │  ")));
    }
    let branch = app.git.as_ref().map(|g| format!("{} ", th.muted(g))).unwrap_or_default();
    let name = app.session_name.as_ref().map(|n| th.muted(&format!(" \u{201c}{n}\u{201d}"))).unwrap_or_default();
    // The folder gives way first, from its front (`…/project`): the branch, the session's
    // name and the router's side are what a glance looks for.
    let room = w.saturating_sub(display_width(&right) + 3 + display_width(&branch) + display_width(&name));
    let place = if room >= 8 { crate::tui::views::status_facts::keep_tail(&app.place, room) } else { app.place.clone() };
    spread(&format!(" {branch}{place}{name}"), &right, w)
}

/// How turns are routed and what each subscription has left:
/// `⇄ auto · claude 32% · codex 71% · gemini cooling 1h20m · orchestrate on`. A plan window
/// at 80% or more is yellow (the router will move off it soon).
fn router_text(app: &App, th: &Theme) -> String {
    let names = Theme::provider_theme(th.0);
    let mut parts = vec![format!("{} {}", th.accent("⇄"), app.route)];
    for p in &app.providers {
        let name = names.name(&p.name);
        let via = p.via.as_deref().filter(|v| *v != p.name).map(|v| th.muted(&format!(" ({v})"))).unwrap_or_default();
        let chip = match (&p.cooling, p.used) {
            (Some(left), _) => format!("{name}{via} {}", th.muted(&format!("cooling {left}"))),
            (None, Some(u)) if u >= 80 => format!("{name}{via} {}", th.warning(&format!("{u}%"))),
            (None, Some(u)) => format!("{name}{via} {}", th.muted(&format!("{u}%"))),
            (None, None) => match &p.credit {
                Some(c) => format!("{name}{via} {}", th.muted(c)),
                None => format!("{name}{via}"),
            },
        };
        parts.push(chip);
    }
    if app.providers.is_empty() {
        parts.push(th.warning("no provider ready · /doctor"));
    }
    if app.orchestrate {
        parts.push(th.muted("orchestrate on"));
    }
    parts.join(&th.muted(" · "))
}

/// The input box and where the cursor is in its rows. Its bottom edge carries the mode.
fn input_box(app: &App, w: usize, th: &Theme) -> (Vec<String>, (usize, usize)) {
    // Inside the borders: `│ ❯ ` before the text, ` │` after.
    let text_w = w.saturating_sub(6).max(1);
    let edge = |s: &str| th.field(s);
    let mut rows = vec![frame('╭', '╮', "", w, th, edge)];
    let label = box_label(app, th);
    let ghost = app.ghost();
    if app.editor.is_empty() {
        // A suggested next prompt takes the placeholder's place; the edge says how to take it.
        let (shown, hint) = match &ghost {
            Some(suggestion) => (truncate_to_width(suggestion, text_w), "→ to use"),
            None if app.working.is_some() => ("Type to queue a message for after this turn".to_string(), ""),
            None => ("Ask anything · / for commands · @ for files".to_string(), ""),
        };
        rows.push(boxed_in(&format!("{} {}", th.accent("❯"), th.muted(&shown)), w, edge));
        rows.push(bottom_edge(hint, &label, w, th));
        return (rows, (1, 4));
    }
    let (lines, (cr, cc)) = app.editor.layout(text_w);
    let first = cr.saturating_sub(INPUT_ROWS - 1);
    for (i, line) in lines.iter().enumerate().skip(first).take(INPUT_ROWS) {
        let lead = if i == 0 { th.accent("❯") } else { " ".to_string() };
        // The ghost text follows the cursor, on its row, cut to what is left of it.
        let rest = match &ghost {
            Some(g) if i == cr => truncate_to_width(g, text_w.saturating_sub(display_width(line))),
            _ => String::new(),
        };
        let rest = if rest.is_empty() { rest } else { th.muted(&rest) };
        rows.push(boxed_in(&format!("{lead} {line}{rest}"), w, edge));
    }
    let hidden = lines.len().saturating_sub(first + INPUT_ROWS);
    let hint = if first > 0 || hidden > 0 { format!("{} lines", lines.len()) } else { String::new() };
    rows.push(bottom_edge(&hint, &label, w, th));
    (rows, (1 + cr - first, 4 + cc))
}

/// What the box's edge says: `NORMAL · auto · edit mode`, the mode in its colour.
fn box_label(app: &App, th: &Theme) -> String {
    let mut parts = Vec::new();
    if let Some(label) = app.vim.as_ref().and_then(|v| v.label()) {
        parts.push(th.strong(label));
    }
    parts.push(th.muted(&app.route));
    parts.push(th.mode(app.mode));
    parts.join(&th.muted(" · "))
}

/// The `/` menu, above the box: a heading row with the selected command's arguments and the
/// position (`/model [target]            1/59`), then the rows, the selected one raised.
fn menu_rows(m: &crate::tui::input::menu::Menu, w: usize, th: &Theme) -> Vec<String> {
    let first = m.first_shown();
    // The label column fits the rows on screen, not the longest label anywhere.
    let label_w = m.items.iter().skip(first).take(MENU_ROWS).map(|i| display_width(&i.label)).max().unwrap_or(0).min(w / 2).max(8);
    let usage = m.items.get(m.selected).filter(|i| !i.usage.is_empty()).map(|i| format!("{} {}", th.accent(&i.label), th.muted(&i.usage))).unwrap_or_default();
    let position = if m.items.len() > MENU_ROWS { th.muted(&format!("{}/{}", m.selected + 1, m.items.len())) } else { String::new() };
    let mut rows = vec![spread(&format!("  {usage}"), &position, w)];
    for (i, item) in m.items.iter().enumerate().skip(first).take(MENU_ROWS) {
        let label = truncate_to_width(&item.label, label_w);
        let pad = " ".repeat(label_w.saturating_sub(display_width(&label)) + 2);
        let detail = truncate_to_width(&item.detail, w.saturating_sub(label_w + 8));
        if i == m.selected {
            let row = format!(" {} {}{pad}{detail}", th.accent("❯"), th.selected(&label));
            let fill = w.saturating_sub(display_width(&row));
            rows.push(th.raised(&format!("{row}{}", " ".repeat(fill))));
        } else {
            rows.push(truncate_to_width(&format!("   {label}{pad}{}", th.muted(&detail)), w));
        }
    }
    rows
}

/// A tool waiting for an answer, in place of the box: a raised card with the question, the
/// tool's input, the numbered choices, and the keys under it.
fn approval_card(a: &crate::tui::app::screen_state::Approval, w: usize, th: &Theme) -> Vec<String> {
    let inner = w.saturating_sub(4).max(1);
    let mut rows = vec![raised_row(&th.strong(&format!("Allow {}?", a.tool)), w, th)];
    let mut body: Vec<String> = Vec::new();
    for line in &a.lines {
        body.extend(wrap_to_width(line, inner));
    }
    let first = a.scroll.min(body.len().saturating_sub(1));
    for line in body.iter().skip(first).take(CARD_INPUT_ROWS) {
        rows.push(raised_row(line, w, th));
    }
    let below = body.len().saturating_sub(first + CARD_INPUT_ROWS);
    if first > 0 || below > 0 {
        rows.push(raised_row(&th.muted(&format!("… {} more lines · pgup/pgdn to scroll", first + below)), w, th));
    }
    for note in &a.notes {
        rows.push(raised_row(&th.muted(note), w, th));
    }
    rows.push(raised_row("", w, th));
    for (i, (_, label)) in a.choices().iter().enumerate() {
        rows.push(raised_row(&choice(i, label, i == a.selected, th), w, th));
    }
    if let Some(note) = &a.note {
        for line in wrap_to_width(note, w.saturating_sub(2)) {
            rows.push(format!("  {}", th.warning(&line)));
        }
    }
    let n = a.choices().len();
    let range = format!("1–{n}");
    rows.push(th.keys(&[(&range, "choose"), ("↑↓", "move"), ("Enter", "confirm"), ("Esc", "no"), ("Ctrl+C", "stop the turn")], w));
    rows
}

/// The key hints under the box, for what the keys do right now; a notice takes their place
/// for a moment.
fn footer(app: &App, w: usize, now: Instant, th: &Theme) -> String {
    if let Some((text, _)) = app.notice.as_ref().filter(|(_, at)| now.duration_since(*at) < crate::tui::app::screen_state::NOTICE_FOR) {
        return truncate_to_width(&format!("  {}", th.warning(text)), w);
    }
    let keys: &[(&str, &str)] = if app.menu.is_some() {
        &[("↑↓", "select"), ("Tab", "complete"), ("Enter", "run"), ("Esc", "close")]
    } else if app.working.is_some() {
        // The working row above already says `esc to stop`.
        &[("Enter", "queue"), ("Shift+Enter", "newline"), ("Shift+Tab", "mode")]
    } else {
        &[("Enter", "send"), ("Shift+Enter", "newline"), ("Shift+Tab", "mode"), ("/", "commands"), ("@", "files")]
    };
    th.keys(keys, w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::console::front::{ApprovalAsk, FrontMsg};
    use crate::tui::input::history::History;
    use crate::tui::render::strip_ansi;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::Path;

    const OFF: Style = Style { enabled: false, terminal: false };
    const ON: Style = Style { enabled: true, terminal: true };

    fn app() -> App {
        App::new(Path::new("/nonexistent").into(), History::default(), crate::approval::domain::mode::Mode::Edit, "auto".into(), Vec::new())
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
    fn the_idle_screen_is_the_header_the_box_with_the_mode_in_its_edge_and_the_keys() {
        let a = app();
        let f = render(&a, 70, Instant::now(), OFF);
        assert!(f.rows[0].starts_with(" /nonexistent") && f.rows[0].ends_with("⇄ auto · no provider ready · /doctor"), "the header: {}", f.rows[0]);
        assert_eq!(f.rows[1], format!("╭{}╮", "─".repeat(67)));
        assert_eq!(f.rows[2], format!("│ ❯ Ask anything · / for commands · @ for files{}│", " ".repeat(21)));
        assert!(f.rows[3].starts_with("╰──") && f.rows[3].ends_with(" auto · edit mode ─╯"), "the mode in the box's edge: {}", f.rows[3]);
        assert_eq!(display_width(&f.rows[3]), 69);
        assert_eq!(f.rows[4], "  Enter:send │ Shift+Enter:newline │ Shift+Tab:mode │ /:commands", "a hint that does not fit is left off whole");
        assert_eq!(render(&a, 90, Instant::now(), OFF).rows[4], "  Enter:send │ Shift+Enter:newline │ Shift+Tab:mode │ /:commands │ @:files");
        assert_eq!(f.rows.len(), 5, "nothing else is boxed or listed: {:?}", f.rows);
        assert_eq!(f.cursor, Some((2, 4)));
    }

    #[test]
    fn a_suggestion_takes_the_placeholders_place_and_ghost_text_follows_the_typed_text() {
        let mut a = app();
        a.suggestion = Some("commit this and tag v1.1.0".into());
        let f = render(&a, 70, Instant::now(), OFF);
        assert!(f.rows[2].starts_with("│ ❯ commit this and tag v1.1.0 "), "{}", f.rows[2]);
        assert!(f.rows[3].starts_with("╰─ → to use ") && f.rows[3].ends_with("edit mode ─╯"), "{}", f.rows[3]);
        assert_eq!(f.cursor, Some((2, 4)), "the cursor sits before the suggestion");

        let mut typing = App::new(Path::new("/nonexistent").into(), History::from_entries(&["fix the parser"]), crate::approval::domain::mode::Mode::Edit, "auto".into(), Vec::new());
        typed(&mut typing, "fix t");
        let f = render(&typing, 70, Instant::now(), OFF);
        assert!(f.rows[2].starts_with("│ ❯ fix the parser "), "{}", f.rows[2]);
        assert_eq!(f.cursor, Some((2, 9)), "the cursor stays after what was typed");
    }

    #[test]
    fn typing_a_slash_opens_the_menu_above_the_box_with_names_and_descriptions() {
        let mut a = app();
        typed(&mut a, "/");
        let f = render(&a, 80, Instant::now(), OFF);
        let s = f.rows.join("\n");
        let box_top = f.rows.iter().position(|r| r.starts_with('╭')).unwrap_or(0);
        let selected = f.rows.iter().position(|r| r.starts_with(" ❯ /model  ")).unwrap_or(usize::MAX);
        assert!(selected < box_top, "the menu sits above the box, the first command selected: {s}");
        assert!(f.rows[1].starts_with("  /model [target]") && f.rows[1].ends_with(&format!("1/{}", crate::tui::render::COMMANDS.len())), "the heading row: {}", f.rows[1]);
        assert!(s.contains("Pin a model or provider, or `auto` to let the router choose"), "names only, so the description fits at 80 columns: {s}");
        assert!(!s.contains("/effort ["), "arguments stay out of the list: {s}");
        assert_eq!(f.rows.iter().filter(|r| r.starts_with("   /") || r.starts_with(" ❯ /")).count(), MENU_ROWS);
        assert!(f.rows.last().is_some_and(|r| r == "  ↑↓:select │ Tab:complete │ Enter:run │ Esc:close"), "the keys say what they do in the menu: {s}");
        a.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), Instant::now());
        assert!(screen(&a, 80).contains("  /provider auto|off"), "the heading follows the selection");
        // With colour the selected row is raised the width of the row.
        let on = render(&a, 80, Instant::now(), ON);
        assert!(on.rows.iter().any(|r| r.contains("\x1b[48;5;236") && strip_ansi(r).starts_with(" ❯ /provider")), "{:?}", on.rows);
    }

    #[test]
    fn a_long_message_wraps_inside_the_box_and_the_cursor_follows() {
        let mut a = app();
        typed(&mut a, &"word ".repeat(20));
        let f = render(&a, 40, Instant::now(), OFF);
        let inside: Vec<_> = f.rows.iter().filter(|r| r.starts_with('│')).collect();
        assert!(inside.len() >= 3, "{:?}", f.rows);
        let (row, col) = f.cursor.unwrap_or_default();
        assert!(row >= 4 && col >= 4, "{:?}", f.cursor);
    }

    #[test]
    fn open_workers_are_listed_above_the_box_with_who_and_how_long() {
        use crate::tui::app::screen_state::WorkerChip;
        let mut a = app();
        let chip = |id: &str, running: bool, age_ms: u64| WorkerChip { id: id.into(), provider: "gemini".into(), model: "argon".into(), running, age_ms, tools: 0, activity: None };
        a.workers = vec![chip("w1", true, 42_000), chip("w2", false, 90_000)];
        let text = screen(&a, 80);
        assert!(text.contains("▾ Workers · 1 running · 1 to review"), "{text}");
        assert!(text.contains("▶ w1  gemini · argon   running 0:42"), "{text}");
        assert!(text.contains("✓ w2  gemini · argon   ready to review"), "{text}");
        a.workers[0].tools = 7;
        a.workers[0].activity = Some("Bash cargo test".into());
        let text = screen(&a, 80);
        assert!(text.contains("running 0:42 · 7 tool calls"), "{text}");
        assert!(text.contains("⎿ Bash cargo test"), "{text}");
        a.workers = (0..6).map(|i| chip(&format!("w{i}"), true, 1000)).collect();
        assert!(screen(&a, 80).contains("… and 2 more"));
        assert!(!screen(&app(), 80).contains("Workers"));
    }

    #[test]
    fn a_running_turn_shows_the_working_row_the_queue_and_the_keys_for_now() {
        let mut a = app();
        a.on_front(FrontMsg::TurnStarted);
        a.on_front(FrontMsg::Label("claude · sonnet-5-5".into()));
        a.on_front(FrontMsg::Activity("Bash cargo test".into()));
        a.queue.push_back("then run clippy".into());
        let s = screen(&a, 80);
        let work = s.lines().nth(1).unwrap_or_default();
        assert!(work.starts_with("  • Running cargo test 0s") && work.ends_with("claude · sonnet-5-5 · esc to stop"), "a still mark without colour: {work}");
        assert!(s.contains("\n  #1 then run clippy\n"), "{s}");
        assert!(s.contains("Type to queue a message for after this turn"), "{s}");
        assert_eq!(s.lines().last(), Some("  Enter:queue │ Shift+Enter:newline │ Shift+Tab:mode"), "Esc is said once, on the working row: {s}");
        assert_eq!(s.matches("esc").count(), 1, "{s}");
        let on = render(&a, 80, Instant::now(), ON).rows[1].clone();
        assert!(on.contains("\x1b[2mRunning") && on.contains("cargo test"), "the verb dim, what it acts on in the accent: {on:?}");
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
    fn an_approval_replaces_the_box_with_a_raised_card_of_numbered_choices() {
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
        assert!(f.rows[1].starts_with("▌ Allow Bash?"), "{s}");
        assert!(s.contains("▌ cargo test --quiet"), "{s}");
        assert!(s.contains("▌ ● 1. Yes "), "{s}");
        assert!(s.contains("▌ ○ 2. Yes, and don't ask again (Bash(cargo test:*))"), "{s}");
        assert!(s.contains("▌ ○ 3. No, and tell the model what to do instead"), "{s}");
        assert!(f.rows.last().is_some_and(|r| r.starts_with("  1–3:choose │ ↑↓:move │ Enter:confirm │ Esc:no")), "{s}");
        assert!(!s.contains('╭'), "no box while the card is up: {s}");
        assert_eq!(f.cursor, None, "no text cursor on a card");
        let on = render(&a, 70, Instant::now(), ON);
        assert!(on.rows[1..7].iter().all(|r| r.contains("\x1b[48;5;236")), "the card is one raised surface: {:?}", on.rows);
    }

    #[test]
    fn every_row_fits_one_column_short_of_the_terminal_at_every_width_and_style() {
        let mut a = app();
        typed(&mut a, "/");
        a.on_front(FrontMsg::TurnStarted);
        a.queue.push_back("日本語のメッセージ ".repeat(10));
        a.git = Some("main ±3".into());
        a.context = Some(91);
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

    fn full_app() -> App {
        let mut a = app();
        a.fullscreen = true;
        a.git = Some("main ±3".into());
        a.providers = vec![crate::tui::app::screen_state::ProviderChip { name: "codex".into(), used: Some(71), cooling: None, via: None, credit: None }];
        a.show("one\ntwo\nthree");
        a
    }

    #[test]
    fn full_screen_is_the_header_on_top_the_chat_full_width_and_the_box_at_the_bottom() {
        let a = full_app();
        let (f, pane) = render_full(&a, 140, 40, Instant::now(), OFF);
        assert_eq!(f.rows.len(), 40);
        assert!(f.rows.iter().all(|r| display_width(r) < 140), "no row reaches the last column");
        assert!(f.rows[0].starts_with(" main ±3 /nonexistent") && f.rows[0].ends_with("⇄ auto · codex 71%"), "{}", f.rows[0]);
        // The chat's newest lines sit just above the box, nothing beside them.
        assert_eq!(f.rows[pane], "three");
        assert!(!f.rows.join("\n").contains("Router"));
        // The cursor is in the box, under the chat.
        let (row, col) = f.cursor.unwrap_or_default();
        assert_eq!(row, pane + 2);
        assert_eq!(col, 4);
        assert!(f.rows[row].contains("Ask anything"), "{}", f.rows[row]);
        assert!(f.rows[39].starts_with("  Enter:send"), "{}", f.rows[39]);
    }

    #[test]
    fn the_answer_being_written_is_drawn_in_the_chat_until_it_is_committed() {
        let mut a = full_app();
        a.live_answer = Some("Hel".into());
        let (f, _) = render_full(&a, 140, 40, Instant::now(), OFF);
        assert!(f.rows.join("\n").contains("Hel"));
        // Scrolled back, the preview stays out of the way.
        a.show(&(1..=30).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\n"));
        a.transcript.scroll_by(5, 10);
        let (f, _) = render_full(&a, 140, 40, Instant::now(), OFF);
        assert!(!f.rows.join("\n").contains("Hel"));
        a.transcript.to_end();
        a.live_answer = None;
        let (f, _) = render_full(&a, 140, 40, Instant::now(), OFF);
        assert!(!f.rows.join("\n").contains("Hel"));
    }

    #[test]
    fn full_screen_survives_a_terminal_shorter_than_the_live_area() {
        let mut a = full_app();
        a.on_front(FrontMsg::TurnStarted);
        for height in [1, 3, 5, 8] {
            let (f, _) = render_full(&a, 120, height, Instant::now(), OFF);
            assert_eq!(f.rows.len(), height, "height {height}");
            if let Some((r, _)) = f.cursor {
                assert!(r < height);
            }
        }
    }

    #[test]
    fn page_up_scrolls_the_chat_back_and_new_output_leaves_the_view_alone() {
        let mut a = full_app();
        a.show(&(0..60).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n"));
        let (f, pane) = render_full(&a, 140, 30, Instant::now(), OFF);
        a.pane_rows = pane;
        assert!(f.rows[pane].contains("line 59"));
        a.on_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), Instant::now());
        let (f, _) = render_full(&a, 140, 30, Instant::now(), OFF);
        assert!(!f.rows[pane].contains("line 59"), "{}", f.rows[pane]);
        a.show("a new line");
        let (f, _) = render_full(&a, 140, 30, Instant::now(), OFF);
        assert!(!f.rows.join("\n").contains("a new line"), "the view stays where it was");
        a.on_key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL), Instant::now());
        let (f, _) = render_full(&a, 140, 30, Instant::now(), OFF);
        assert!(f.rows[pane].contains("a new line"));
    }

    #[test]
    fn the_header_shows_each_subscription_and_what_it_has_left() {
        use crate::tui::app::screen_state::ProviderChip;
        let mut a = app();
        a.providers = vec![
            ProviderChip { name: "claude".into(), used: Some(32), cooling: None, via: None, credit: None },
            ProviderChip { name: "codex".into(), used: Some(85), cooling: None, via: None, credit: None },
            ProviderChip { name: "gemini".into(), used: None, cooling: Some("1h20m".into()), via: Some("agy".into()), credit: None },
            ProviderChip { name: "deepseek".into(), used: None, cooling: None, via: Some("deepseek".into()), credit: Some("$4.20 left".into()) },
        ];
        a.orchestrate = true;
        let header = render(&a, 140, Instant::now(), OFF).rows[0].clone();
        assert!(header.ends_with("⇄ auto · claude 32% · codex 85% · gemini (agy) cooling 1h20m · deepseek $4.20 left · orchestrate on"), "the CLI is named only when it differs: {header}");
        let on = render(&a, 140, Instant::now(), ON).rows[0].clone();
        assert!(on.contains("\x1b[33m85%"), "a plan window at 80% or more is yellow: {on:?}");
    }

    #[test]
    fn the_header_names_the_branch_the_folder_and_the_context_window() {
        let mut a = app();
        a.git = Some("main ±3".into());
        a.session_name = Some("parser".into());
        a.context = Some(30);
        let header = render(&a, 140, Instant::now(), OFF).rows[0].clone();
        assert!(header.starts_with(" main ±3 /nonexistent \u{201c}parser\u{201d}"), "{header}");
        assert!(header.ends_with("  │  context 30%"), "{header}");
        a.context = Some(85);
        assert!(render(&a, 140, Instant::now(), OFF).rows[0].ends_with("context 85% · /compact"));
        // Narrow, the folder is cut before the router's side gives way.
        let narrow = render(&a, 80, Instant::now(), OFF).rows[0].clone();
        assert!(narrow.contains('…') && narrow.contains("⇄ auto"), "{narrow}");
        // A long folder gives way from its front; the session's name stays.
        a.place = "/private/var/folders/n0/j5fhc3d939n_f150v/T/jevux-rename/repo".into();
        a.context = None;
        let long = render(&a, 100, Instant::now(), OFF).rows[0].clone();
        assert!(long.contains(" …/") && long.contains("/jevux-rename/repo \u{201c}parser\u{201d}") && long.ends_with("⇄ auto · no provider ready · /doctor"), "{long}");
    }

    #[test]
    fn a_notice_takes_the_place_of_the_keys_and_the_mode_is_named_not_only_coloured() {
        let mut a = app();
        a.notice = Some(("Press Ctrl-C again to exit".into(), Instant::now()));
        let s = screen(&a, 80);
        assert_eq!(s.lines().last(), Some("  Press Ctrl-C again to exit"), "{s}");
        assert!(s.contains("edit mode ─╯"), "{s}");
        a.mode = crate::approval::domain::mode::Mode::Plan;
        assert!(screen(&a, 80).contains(" auto · plan mode ─╯"));
    }
}
