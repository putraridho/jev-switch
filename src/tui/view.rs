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
use crate::workstation::panel::{compose, panel_rows, Section, PANEL_WIDTH, SPLIT_MIN_WIDTH};
use crate::uifmt::{answer_panel, answer_text, display_width,truncate_to_width, wrap_to_width, Spinner, Style};

/// Below the side-by-side width, the panel stacks under the box down to this width; narrower
/// than that, the one-row router summary is all that fits.
const STACK_MIN_WIDTH: usize = 40;
/// Full screen: a terminal shorter than this keeps the panel off (the chat needs the rows).
const STACK_MIN_HEIGHT: usize = 30;
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
    fn border(&self, s: &str) -> String {
        self.0.chrome(s)
    }
    fn selected(&self, s: &str) -> String {
        self.0.bold(&self.0.accent(s))
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
    truncate_to_width(&format!("{} {} {}", style.accent(spinner), style.bold(&what), style.dim(&format!("· {time}{who}"))), width)
}

/// The most workers listed by name above the box; the rest are counted.
const WORKER_ROWS: usize = 4;

/// What the planner's workers are doing, above the box, in a titled box of their own:
/// `╭─ ⇉ Workers · 1 running · 1 to review ─╮`, then a row each (`▶ w1  gemini · argon
/// running 0:42`, `✓ w2  codex  ready to review`). Nothing when no worker is open.
fn workers_rows(app: &App, w: usize, style: Style) -> Vec<String> {
    if app.workers.is_empty() {
        return Vec::new();
    }
    let th = Theme(style);
    let names = Theme::provider_theme(style);
    let running = app.workers.iter().filter(|x| x.running).count();
    let waiting = app.workers.len() - running;
    let mut title = String::from("⇉ Workers");
    if running > 0 {
        title.push_str(&format!(" · {running} running"));
    }
    if waiting > 0 {
        title.push_str(&format!(" · {waiting} to review"));
    }
    let mut rows = vec![border('╭', '╮', &title, w, &th)];
    for wk in app.workers.iter().take(WORKER_ROWS) {
        let (glyph, state) = if wk.running {
            (style.accent("▶"), style.dim(&format!("running {}", clock(wk.age_ms))))
        } else {
            (style.yellow("✓"), style.yellow("ready to review"))
        };
        let who = if wk.model.is_empty() { names.name(&wk.provider) } else { format!("{} {} {}", names.name(&wk.provider), style.dim("·"), wk.model) };
        let calls = if wk.running && wk.tools > 0 { style.dim(&format!(" · {} tool calls", wk.tools)) } else { String::new() };
        rows.push(boxed(&format!("{glyph} {}  {who}   {state}{calls}", style.bold(&wk.id)), w, &th));
        // What a running worker is doing right now, under it.
        if let Some(doing) = wk.activity.as_deref().filter(|_| wk.running) {
            rows.push(boxed(&format!("  {}", th.muted(&format!("⎿ {doing}"))), w, &th));
        }
    }
    if app.workers.len() > WORKER_ROWS {
        rows.push(boxed(&style.dim(&format!("… and {} more", app.workers.len() - WORKER_ROWS)), w, &th));
    }
    rows.push(border('╰', '╯', "", w, &th));
    rows
}

/// `0:42`, `12:05`: a worker's age.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Where the side panel goes relative to the live area.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    /// Inline: beside the box when wide, under it when narrow.
    Auto,
    /// Full screen, wide: the panel is its own column, so the live area leaves it out.
    Separate,
    /// Too short a terminal to stack the panel: only the one-row router summary.
    Compact,
}

/// Draw the live area for `width` terminal columns at time `now`.
pub fn render(app: &App, width: usize, now: Instant, style: Style) -> Frame {
    live(app, width, now, style, Side::Auto)
}

fn live(app: &App, width: usize, now: Instant, style: Style, place: Side) -> Frame {
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
    rows.extend(workers_rows(app, w, style));
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
        // On a wide terminal the side panel (router, changed files) sits beside the box.
        let side = if place == Side::Auto && w >= SPLIT_MIN_WIDTH { panel_rows(&panel_sections(app, style), PANEL_WIDTH, style) } else { Vec::new() };
        let main_w = if side.is_empty() { w } else { w - PANEL_WIDTH - 3 };
        let (box_rows, at) = input_box(app, main_w, &th);
        let top = rows.len();
        let mut main = box_rows;
        cursor = Some((top + at.0, at.1));
        if let Some(m) = &app.menu {
            main.extend(menu_rows(m, main_w, &th));
        }
        if side.is_empty() {
            rows.extend(main);
            // Too narrow to sit beside the box, the panel moves under it, as wide as the
            // box; it says what the router row would, so that row steps aside.
            let stacked = if place == Side::Auto && w >= STACK_MIN_WIDTH { panel_rows(&[stacked_section(app, style)], w, style) } else { Vec::new() };
            if stacked.is_empty() {
                // Beside it in its own column, the panel says what this row would.
                if place != Side::Separate {
                    rows.push(router_row(app, w, &th));
                }
            } else {
                rows.extend(stacked);
            }
            rows.push(footer(app, w, now, &th));
        } else {
            // Beside the panel, the router row and footer sit right under the box: the panel is
            // taller than the box, and at the bottom of it they left a gap.
            main.push(router_row(app, main_w, &th));
            main.push(footer(app, main_w, now, &th));
            rows.extend(compose(&main, &side, w, style));
        }
    }
    Frame { rows, cursor }
}

/// The panel's sections as one box, for under the input box: the router's rows, then the
/// context meter and the branch on one row. Fewer rows than three boxes of their own.
fn stacked_section(app: &App, style: Style) -> Section {
    let mut sections = panel_sections(app, style).into_iter();
    let mut rows = sections.next().map(|s| s.rows).unwrap_or_default();
    let rest: Vec<String> = sections.flat_map(|s| s.rows).collect();
    if !rest.is_empty() {
        rows.push(rest.join(&style.dim("  ·  ")));
    }
    Section { title: "Router".into(), rows }
}

/// The sidebar at full height: its boxes from the top, the last one stretched down to the
/// bottom row, so the column reads as one tall pane beside the chat. Exactly `height` rows.
fn sidebar(app: &App, width: usize, height: usize, style: Style) -> Vec<String> {
    let mut rows = panel_rows(&panel_sections(app, style), width, style);
    if rows.is_empty() {
        let empty = [Section { title: "jev-code".into(), rows: vec![String::new()] }];
        rows = panel_rows(&empty, width, style);
    }
    rows.truncate(height);
    // The last row of the last box is its bottom border: stretch it.
    if rows.len() < height {
        let bottom = rows.pop().unwrap_or_default();
        let inner = width.saturating_sub(4);
        let blank = format!("{} {} {}", style.chrome("│"), " ".repeat(inner), style.chrome("│"));
        while rows.len() + 1 < height {
            rows.push(blank.clone());
        }
        rows.push(bottom);
    }
    rows
}

/// The whole terminal, `cols` by `height`: the chat on top, the input box under it, and the
/// side panel as a full-height column on the right. Below [`SPLIT_MIN_WIDTH`] columns the
/// panel moves under the input box instead. Returns the frame (exactly `height` rows) and how
/// many rows the chat pane has.
pub fn render_full(app: &App, cols: usize, height: usize, now: Instant, style: Style) -> (Frame, usize) {
    let w = cols.saturating_sub(1).max(20);
    let wide = w >= SPLIT_MIN_WIDTH;
    let main_w = if wide { w - PANEL_WIDTH - 3 } else { w };
    let mut live = live(app, main_w, now, style, if wide { Side::Separate } else if height >= STACK_MIN_HEIGHT { Side::Auto } else { Side::Compact });
    // A terminal too short for the live area keeps its bottom rows (the box and the footer).
    if live.rows.len() > height {
        let cut = live.rows.len() - height;
        live.rows.drain(..cut);
        live.cursor = live.cursor.and_then(|(r, c)| r.checked_sub(cut).map(|r| (r, c)));
    }
    let chat_h = height - live.rows.len();
    let mut left = chat_rows(app, main_w, chat_h, style);
    left.extend(live.rows);
    let cursor = live.cursor.map(|(r, c)| (r + chat_h, c));
    let rows = if wide {
        let side = sidebar(app, PANEL_WIDTH, height, style);
        left.iter()
            .zip(side.iter().chain(std::iter::repeat(&String::new())))
            .map(|(l, r)| format!("{l}{}   {r}", " ".repeat(main_w.saturating_sub(display_width(l)))))
            .collect()
    } else {
        left
    };
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

/// A usage meter, `▰▰▰▱▱▱▱▱`, filled in proportion to `pct` and coloured by how close to the
/// limit it is (green, then yellow, then red). The shapes differ, so it reads without colour.
fn meter(pct: u8, cells: usize, style: Style) -> String {
    let pct = pct.min(100);
    let filled = (usize::from(pct) * cells + 50) / 100;
    let on = "▰".repeat(filled);
    let on = style.level(pct, &on);
    format!("{on}{}", style.chrome(&"▱".repeat(cells - filled)))
}

/// What the side panel lists: each subscription's plan use as a meter (or its cooldown), how
/// full the context window is, and the git state.
fn panel_sections(app: &App, style: Style) -> Vec<Section> {
    let th = Theme::provider_theme(style);
    let router = app
        .providers
        .iter()
        .map(|p| {
            let name = format!("{}{}", th.name(&p.name), " ".repeat(8usize.saturating_sub(display_width(&p.name))));
            match (&p.cooling, p.used) {
                (Some(left), _) => format!("{name} {}", style.yellow(&format!("cooling {left}"))),
                (None, Some(u)) => format!("{name} {} {u:>3}%", meter(u, 8, style)),
                (None, None) => match p.via.as_deref().filter(|v| *v != p.name) {
                    Some(v) => format!("{name} {}", style.dim(&format!("ready · {v}"))),
                    None => format!("{name} {}", style.dim("ready")),
                },
            }
        })
        .collect();
    let context = app.context.map(|c| vec![format!("{} {c:>3}%", meter(c, 14, style))]).unwrap_or_default();
    vec![
        Section { title: "Router".into(), rows: router },
        Section { title: "Context".into(), rows: context },
        Section { title: "Branch".into(), rows: app.git.iter().cloned().collect() },
    ]
}

/// The input box and where the cursor is in its rows.
fn input_box(app: &App, w: usize, th: &Theme) -> (Vec<String>, (usize, usize)) {
    // Inside the borders: `│ › ` before the text, ` │` after.
    let text_w = w.saturating_sub(6).max(1);
    let mut rows = vec![border('╭', '╮', "", w, th)];
    let ghost = app.ghost();
    if app.editor.is_empty() {
        // A suggested next prompt takes the placeholder's place; the border says how to take it.
        let (shown, hint) = match &ghost {
            Some(suggestion) => (truncate_to_width(suggestion, text_w), "→ to use"),
            None if app.working.is_some() => ("Type to queue a message for after this turn".to_string(), ""),
            None => ("Ask anything · / for commands · @ for files".to_string(), ""),
        };
        rows.push(boxed(&format!("{} {}", th.accent("›"), th.muted(&shown)), w, th));
        rows.push(border('╰', '╯', hint, w, th));
        return (rows, (1, 4));
    }
    let (lines, (cr, cc)) = app.editor.layout(text_w);
    let first = cr.saturating_sub(INPUT_ROWS - 1);
    for (i, line) in lines.iter().enumerate().skip(first).take(INPUT_ROWS) {
        let lead = if i == 0 { th.accent("›") } else { " ".to_string() };
        // The ghost text follows the cursor, on its row, cut to what is left of it.
        let rest = match &ghost {
            Some(g) if i == cr => truncate_to_width(g, text_w.saturating_sub(display_width(line))),
            _ => String::new(),
        };
        let rest = if rest.is_empty() { rest } else { th.muted(&rest) };
        rows.push(boxed(&format!("{lead} {line}{rest}"), w, th));
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
    if let Some(label) = app.vim.as_ref().and_then(|v| v.label()) {
        left.push_str(&th.strong(&format!(" · {label}")));
    }
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
    fn a_suggestion_takes_the_placeholders_place_and_ghost_text_follows_the_typed_text() {
        let mut a = app();
        a.suggestion = Some("commit this and tag v1.1.0".into());
        let f = render(&a, 70, Instant::now(), OFF);
        assert!(f.rows[1].starts_with("│ › commit this and tag v1.1.0 "), "{}", f.rows[1]);
        assert!(f.rows[2].contains("→ to use"), "{}", f.rows[2]);
        assert_eq!(f.cursor, Some((1, 4)), "the cursor sits before the suggestion");

        let mut typing = App::new(Path::new("/nonexistent").into(), History::from_entries(&["fix the parser"]), crate::mode::Mode::Edit, "auto".into(), Vec::new());
        typed(&mut typing, "fix t");
        let f = render(&typing, 70, Instant::now(), OFF);
        assert!(f.rows[1].starts_with("│ › fix the parser "), "{}", f.rows[1]);
        assert_eq!(f.cursor, Some((1, 9)), "the cursor stays after what was typed");
    }

    #[test]
    fn typing_a_slash_shows_the_command_menu_with_descriptions() {
        let mut a = app();
        typed(&mut a, "/");
        let s = screen(&a, 80);
        assert!(s.contains("│ › /"), "{s}");
        assert!(s.contains("  › /model [target]"), "the first command is selected: {s}");
        assert!(s.contains("Pin a model or provider"), "{s}");
        assert!(s.contains(&format!("1/{} · ↑↓ more", crate::uifmt::COMMANDS.len())), "{s}");
        assert!(s.contains(" · ↑↓ more · tab complete · enter run · esc close"), "{s}");
        assert!(s.contains(&format!("1/{} · ↑↓ more · tab complete · enter run · esc close", crate::uifmt::COMMANDS.len())), "{s}");
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
    fn open_workers_are_listed_above_the_box_with_who_and_how_long() {
        use super::super::app::WorkerChip;
        let mut a = app();
        let chip = |id: &str, running: bool, age_ms: u64| WorkerChip { id: id.into(), provider: "gemini".into(), model: "argon".into(), running, age_ms, tools: 0, activity: None };
        a.workers = vec![chip("w1", true, 42_000), chip("w2", false, 90_000)];
        let text = screen(&a, 80);
        assert!(text.contains("⇉ Workers · 1 running · 1 to review"), "{text}");
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
    fn a_wide_terminal_shows_the_side_panel_beside_the_box_and_a_narrow_one_does_not() {
        let mut a = app();
        a.git = Some("main ±3".into());
        a.providers = vec![crate::tui::app::ProviderChip { name: "codex".into(), used: Some(71), cooling: None, via: None }];
        let wide = render(&a, 140, Instant::now(), OFF);
        let text = wide.rows.join("\n");
        assert!(wide.rows[0].contains("Router"), "{text}");
        assert!(text.contains("codex 71%") && text.contains("main ±3"), "{text}");
        assert!(wide.rows.iter().all(|r| display_width(r) < 140));
        // Too narrow to sit beside the box, the panel stacks under it.
        let narrow = screen(&a, 80);
        let (box_end, router) = (narrow.lines().position(|l| l.starts_with('╰')), narrow.lines().position(|l| l.contains("╭─ Router")));
        assert!(box_end.is_some() && router > box_end, "{narrow}");
        assert!(narrow.contains("codex    ▰▰▰▰▰▰▱▱  71%") && narrow.contains("main ±3"), "{narrow}");
        assert!(!narrow.contains("⇄ auto"), "the panel replaces the router row: {narrow}");
        // Narrower than that, the one-row router summary is what fits.
        assert!(screen(&a, 30).contains("⇄ auto") && !screen(&a, 30).contains("Router"));
    }

    fn full_app() -> App {
        let mut a = app();
        a.fullscreen = true;
        a.git = Some("main ±3".into());
        a.providers = vec![crate::tui::app::ProviderChip { name: "codex".into(), used: Some(71), cooling: None, via: None }];
        a.show("one\ntwo\nthree");
        a
    }

    #[test]
    fn full_screen_is_exactly_the_terminal_with_the_sidebar_at_full_height_beside_the_chat() {
        let a = full_app();
        let (f, pane) = render_full(&a, 140, 40, Instant::now(), OFF);
        assert_eq!(f.rows.len(), 40);
        assert!(f.rows.iter().all(|r| display_width(r) < 140), "no row reaches the last column");
        // The chat's newest lines sit just above the box, the sidebar spans every row.
        assert_eq!(f.rows[pane - 1].trim_end().split("   ").next().unwrap_or_default().trim_end(), "three");
        assert!(f.rows[0].contains("╭─ Router") && f.rows[0].starts_with(' '), "{}", f.rows[0]);
        assert!(f.rows[39].ends_with('╯'), "the sidebar's last box ends on the last row: {}", f.rows[39]);
        // The cursor is in the box, under the chat.
        let (row, col) = f.cursor.unwrap_or_default();
        assert_eq!(row, pane + 1);
        assert_eq!(col, 4);
        assert!(f.rows[row].contains("Ask anything") || f.rows[row].contains('›'), "{}", f.rows[row]);
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
    fn full_screen_below_the_split_width_puts_the_panel_under_the_box_when_there_is_height() {
        let a = full_app();
        let (tall, pane_tall) = render_full(&a, 80, 40, Instant::now(), OFF);
        let text = tall.rows.join("\n");
        assert_eq!(tall.rows.len(), 40);
        assert!(text.contains("╭─ Router") && !text.contains("⇄ auto"), "{text}");
        let box_row = tall.rows.iter().position(|r| r.contains("Ask anything")).unwrap_or(0);
        let router_row = tall.rows.iter().position(|r| r.contains("╭─ Router")).unwrap_or(0);
        assert!(router_row > box_row && box_row >= pane_tall, "{text}");
        // A short terminal keeps the rows for the chat: the one-row summary instead.
        let (short, _) = render_full(&a, 80, 20, Instant::now(), OFF);
        assert_eq!(short.rows.len(), 20);
        let text = short.rows.join("\n");
        assert!(text.contains("⇄ auto") && !text.contains("╭─ Router"), "{text}");
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
        assert!(f.rows[pane - 1].contains("line 59"));
        a.on_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), Instant::now());
        let (f, _) = render_full(&a, 140, 30, Instant::now(), OFF);
        assert!(!f.rows[pane - 1].contains("line 59"), "{}", f.rows[pane - 1]);
        a.show("a new line");
        let (f, _) = render_full(&a, 140, 30, Instant::now(), OFF);
        assert!(!f.rows.join("\n").contains("a new line"), "the view stays where it was");
        a.on_key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL), Instant::now());
        let (f, _) = render_full(&a, 140, 30, Instant::now(), OFF);
        assert!(f.rows[pane - 1].contains("a new line"));
    }

    #[test]
    fn the_router_row_shows_each_subscription_and_what_it_has_left() {
        use crate::tui::app::ProviderChip;
        let mut a = app();
        a.providers = vec![
            ProviderChip { name: "claude".into(), used: Some(32), cooling: None, via: None },
            ProviderChip { name: "codex".into(), used: Some(85), cooling: None, via: None },
            ProviderChip { name: "gemini".into(), used: None, cooling: Some("1h20m".into()), via: None },
        ];
        a.orchestrate = true;
        let s = screen(&a, 120);
        assert!(s.contains("  ⇄ auto · claude 32% · codex 85% · gemini cooling 1h20m · orchestrate on"), "{s}");
        a.providers.clear();
        assert!(screen(&a, 120).contains("⇄ auto · no provider ready · /doctor"));
    }

    #[test]
    fn a_ready_row_names_the_cli_only_when_it_differs_from_the_provider() {
        use crate::tui::app::ProviderChip;
        let style = crate::uifmt::Style::off();
        let rows = |via: Option<&str>| {
            let mut a = app();
            a.providers = vec![ProviderChip { name: "gemini".into(), used: None, cooling: None, via: via.map(String::from) }];
            panel_sections(&a, style).remove(0).rows.join("\n")
        };
        assert!(rows(Some("agy")).contains("ready · agy"));
        assert!(!rows(Some("gemini")).contains('·'));
        assert!(!rows(None).contains('·'));
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
