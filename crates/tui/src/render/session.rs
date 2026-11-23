//! The screens of the session commands: `/cost`, `/context`, `/why`, `/config` and
//! `/terminal-setup`. Plain facts in, text out.

use crate::render::route::RouteFacts;
use crate::render::style::Style;
use crate::render::text::{human_count, human_duration};
use crate::render::width::{display_width, pad_to_width};

const LABEL_COLS: usize = 12;

fn row(label: &str, value: &str, style: Style) -> String {
    format!("  {}{value}", style.dim(&pad_to_width(label, LABEL_COLS)))
}

// ----------------------------------------------------------------------- /cost

/// Tokens and time of one model (or of all of them) over the session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CostRow {
    pub model: String,
    pub turns: usize,
    pub in_tokens: u64,
    pub cached_tokens: u64,
    pub out_tokens: u64,
    pub elapsed_ms: u64,
    pub unreported_turns: usize,
}

/// `/cost`: a table by model, the total, and how full each plan window is. The CLIs run on
/// subscriptions, so the plan windows are the real cost; tokens show what used them.
pub fn cost_text(rows: &[CostRow], total: &CostRow, plans: &[(String, f64)], style: Style) -> String {
    if rows.is_empty() {
        return format!("  {}", style.dim("No turns in this session yet."));
    }
    let model_col = rows.iter().map(|r| display_width(&r.model)).max().unwrap_or(0).max(5) + 2;
    let cells = |r: &CostRow| {
        let turns = if r.turns == 1 { "1 turn".to_string() } else { format!("{} turns", r.turns) };
        let count = |tokens| {
            if r.unreported_turns == r.turns {
                "unknown".to_string()
            } else if r.unreported_turns > 0 {
                format!(">={}", human_count(tokens))
            } else {
                human_count(tokens)
            }
        };
        format!(
            "{:<8}  in {:>6} (cached {:>6})  out {:>6}  {:>7}",
            turns,
            count(r.in_tokens),
            count(r.cached_tokens),
            count(r.out_tokens),
            human_duration(r.elapsed_ms)
        )
    };
    let mut lines = vec![style.bold("This session")];
    for r in rows {
        lines.push(format!("  {}{}", pad_to_width(&r.model, model_col), cells(r)));
    }
    if rows.len() > 1 {
        lines.push(format!("  {}{}", style.bold(&pad_to_width("total", model_col)), cells(total)));
    }
    if !plans.is_empty() {
        lines.push(String::new());
        lines.push(style.bold("Plan windows"));
        for (provider, used) in plans {
            let pct = format!("{:.0}%", used * 100.0);
            let pct = if *used >= 0.8 { style.yellow(&pct) } else { pct };
            lines.push(format!("  {}{pct} used", pad_to_width(provider, model_col)));
        }
    }
    lines.join("\n")
}

// -------------------------------------------------------------------- /context

/// What `/context` knows about the last turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextFacts {
    /// `provider · model` of the last turn.
    pub model: String,
    /// Input tokens of the last request, when the provider reports them.
    pub used: Option<u64>,
    /// The model's context window, when the catalog knows it.
    pub window: Option<u64>,
    /// Turns in this session.
    pub turns: usize,
    /// Notes waiting to go with the next message (`!command` output, a summary).
    pub carried: usize,
}

/// Above this share of the window, `/context` suggests `/compact`.
pub const COMPACT_SUGGEST_AT: f64 = 0.7;

/// A bar of `cells` cells, `share` (0..1) of them full.
fn bar(share: f64, cells: usize) -> String {
    let full = ((share.clamp(0.0, 1.0) * cells as f64).round() as usize).min(cells);
    format!("{}{}", "█".repeat(full), "░".repeat(cells - full))
}

/// `/context`: how much of the window the last turn used, as a number and a bar.
pub fn context_text(f: Option<&ContextFacts>, style: Style) -> String {
    let Some(f) = f else {
        return format!("  {}", style.dim("No turn has run yet, so nothing is in context."));
    };
    let mut lines = vec![style.bold("Context")];
    match (f.used, f.window) {
        (Some(used), Some(window)) if window > 0 => {
            let share = used as f64 / window as f64;
            let pct = format!("{:.0}%", share * 100.0);
            let painted = if share >= COMPACT_SUGGEST_AT { style.yellow(&pct) } else { pct };
            let value = format!("{} of {} tokens ({painted})  {}", human_count(used), human_count(window), style.dim(&bar(share, 20)));
            lines.push(row("used", &value, style));
            lines.push(row("model", &f.model, style));
            lines.push(row("turns", &f.turns.to_string(), style));
            if f.carried > 0 {
                lines.push(row("waiting", &format!("{} note(s) go with your next message", f.carried), style));
            }
            if share >= COMPACT_SUGGEST_AT {
                lines.push(String::new());
                lines.push(format!("  {}", style.dim("Getting full: /compact summarises the conversation and starts fresh from the summary.")));
            }
        }
        (Some(used), _) => {
            lines.push(row("used", &format!("{} tokens", human_count(used)), style));
            lines.push(row("model", &format!("{} {}", f.model, style.dim("(window size unknown)")), style));
            lines.push(row("turns", &f.turns.to_string(), style));
        }
        (None, _) => {
            lines.push(row("used", "unknown (provider did not report context occupancy)", style));
            if let Some(window) = f.window.filter(|window| *window > 0) {
                lines.push(row("window", &format!("{} tokens", human_count(window)), style));
            }
            lines.push(row("model", &f.model, style));
            lines.push(row("turns", &f.turns.to_string(), style));
        }
    }
    lines.join("\n")
}

// ------------------------------------------------------------------------ /why

/// `/why`: the model of the last turn and the router's reason, in full.
pub fn why_text(route: Option<&RouteFacts>, pin: &str, routing_on: bool, style: Style) -> String {
    let Some(r) = route else {
        return format!("  {}", style.dim("No turn has been routed yet."));
    };
    let model = if r.model.is_empty() { r.provider.clone() } else { format!("{} · {}", r.provider, r.model) };
    let mut lines = vec![style.bold("Last route"), row("model", &model, style)];
    let reason = if r.reason.is_empty() { r.task.clone() } else { r.reason.clone() };
    if !reason.is_empty() {
        lines.push(row("reason", &reason, style));
    }
    let mut notes: Vec<&str> = Vec::new();
    if r.switched_provider {
        notes.push("switched provider (it was caught up on what the others did)");
    }
    if r.dropped_tier {
        notes.push("a lower tier than the task asked for (the stronger ones were busy or out of quota)");
    }
    for n in notes {
        lines.push(row("note", n, style));
    }
    let setting = match (pin, routing_on) {
        ("auto", true) => "auto: the router picks per turn (/model to pin one)".to_string(),
        ("auto", false) => "routing off: Claude only (/provider auto to route)".to_string(),
        (p, _) => format!("pinned to {p} (/model auto to let the router choose)"),
    };
    lines.push(row("setting", &setting, style));
    lines.join("\n")
}

// --------------------------------------------------------------------- /config

/// One setting for `/config`: its name, its value as shown, and how to change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRow {
    pub name: String,
    pub value: String,
    pub change: String,
}

/// `/config`: the session's settings, then the environment ones, then where they live.
pub fn config_text(session: &[ConfigRow], env: &[ConfigRow], files: &[(String, String)], style: Style) -> String {
    let name_col = session.iter().chain(env).map(|r| display_width(&r.name)).max().unwrap_or(0) + 2;
    let value_col = session.iter().chain(env).map(|r| display_width(&r.value)).max().unwrap_or(0).min(36) + 2;
    let draw = |r: &ConfigRow| {
        let change = if r.change.is_empty() { String::new() } else { style.dim(&r.change) };
        format!("  {}{}{change}", pad_to_width(&r.name, name_col), pad_to_width(&r.value, value_col)).trim_end().to_string()
    };
    let mut lines = vec![style.bold("This session")];
    lines.extend(session.iter().map(draw));
    lines.push(String::new());
    lines.push(style.bold("From the environment or ~/.jev-router.env"));
    lines.extend(env.iter().map(draw));
    lines.push(String::new());
    lines.push(style.bold("Files"));
    let file_col = files.iter().map(|(n, _)| display_width(n)).max().unwrap_or(0) + 2;
    for (name, path) in files {
        lines.push(format!("  {}{path}", pad_to_width(name, file_col)));
    }
    lines.join("\n")
}

// ------------------------------------------------------------- /terminal-setup

/// What `/terminal-setup` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSetupFacts {
    /// The terminal's name; `None` when it could not be told.
    pub terminal: Option<String>,
    pub file: String,
    pub snippet: String,
    pub reload: String,
    pub can_install: bool,
}

/// `/terminal-setup`: what to add where, and how to let jev-code do it.
pub fn terminal_setup_text(f: &TerminalSetupFacts, style: Style) -> String {
    let mut lines = vec![style.bold("Shift-Enter for a new line")];
    lines.push(format!(
        "  {}",
        style.dim("Most terminals send the same key for Shift-Enter as for Enter. Make Shift-Enter send Esc+Enter, which jev-code reads as a new line.")
    ));
    lines.push(format!("  {}", style.dim("Alt-Enter, and \\ then Enter, work already in every terminal.")));
    lines.push(String::new());
    let Some(name) = &f.terminal else {
        lines.push("  This terminal could not be told from its environment.".to_string());
        lines.push(format!("  {}", style.dim("kitty, Ghostty, Alacritty, foot, WezTerm and VS Code are known; map Shift+Enter to send \\x1b\\r in yours.")));
        return lines.join("\n");
    };
    lines.push(format!("  {} {}", style.bold(name), style.dim(&format!("· add to {}", f.file))));
    lines.push(String::new());
    for l in f.snippet.lines() {
        lines.push(format!("    {}", style.cyan(l)));
    }
    lines.push(String::new());
    if f.can_install {
        lines.push(format!("  {} {}", style.dim("Let jev-code add it (with a backup):"), style.cyan("/terminal-setup install")));
    }
    lines.push(format!("  {}", style.dim(&format!("Then: {}.", f.reload))));
    lines.join("\n")
}

// ------------------------------------------------------------------------ /mcp

/// `/mcp`: show configured MCP servers across ready providers.
pub fn mcp_text(servers: &[jev_backend::infrastructure::mcp_discovery::McpRow], style: Style) -> String {
    if servers.is_empty() {
        return format!(
            "{}\n\n  {}",
            style.bold("MCP servers"),
            style.dim("No MCP servers configured in this project or in ~/.claude.json, ~/.codex/config.toml, ~/.gemini/config/mcp_config.json.")
        );
    }

    let name_col = servers.iter().map(|r| display_width(&r.name)).max().unwrap_or(4).max(4) + 2;
    let scope_col = servers.iter().map(|r| display_width(&r.scope.to_string())).max().unwrap_or(8).max(8) + 2;
    let transport_col = servers.iter().map(|r| display_width(&r.transport.to_string())).max().unwrap_or(9).max(9) + 2;

    let header = format!(
        "  {}{}{}{}",
        style.dim(&pad_to_width("Name", name_col)),
        style.dim(&pad_to_width("Provider", scope_col)),
        style.dim(&pad_to_width("Transport", transport_col)),
        style.dim("Target")
    );

    let mut lines = vec![style.bold("MCP servers"), String::new(), header];
    for r in servers {
        let line = format!(
            "  {}{}{}{}",
            pad_to_width(&r.name, name_col),
            pad_to_width(&r.scope.to_string(), scope_col),
            pad_to_width(&r.transport.to_string(), transport_col),
            r.target
        );
        lines.push(line);
    }

    lines.push(String::new());
    lines.push(format!("  {}", style.dim("To sign in to an HTTP server: /mcp login <server>")));

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::test_facts::*;

    fn cost_row(model: &str, turns: usize) -> CostRow {
        CostRow { model: model.into(), turns, in_tokens: 45_000, cached_tokens: 40_000, out_tokens: 189, elapsed_ms: 14_000, unreported_turns: 0 }
    }

    #[test]
    fn cost_lists_models_a_total_and_plan_windows() {
        let rows = [cost_row("claude · sonnet", 2), cost_row("codex · luna", 1)];
        let text = cost_text(&rows, &cost_row("total", 3), &[("claude".into(), 0.42), ("codex".into(), 0.85)], OFF);
        assert!(text.contains("claude · sonnet  2 turns") && text.contains("in    45K (cached    40K)  out    189"), "{text}");
        assert!(text.contains("total") && text.contains("3 turns"), "{text}");
        assert!(text.contains("Plan windows") && text.contains("42% used") && text.contains("85% used"), "{text}");
        no_esc(&text);
        assert!(cost_text(&rows[..1], &rows[0], &[], OFF).lines().all(|l| !l.contains("total")));
        assert!(cost_text(&[], &CostRow::default(), &[], OFF).contains("No turns"));
        assert!(cost_text(&rows, &rows[0], &[("codex".into(), 0.85)], ON).contains("\x1b[33m85%"));
    }

    #[test]
    fn cost_marks_unreported_consumption_and_mixed_totals() {
        let unknown = CostRow { unreported_turns: 1, ..cost_row("gemini", 1) };
        let text = cost_text(std::slice::from_ref(&unknown), &unknown, &[], OFF);
        assert!(text.contains("unknown") && !text.contains("45K"), "{text}");
        let mixed = CostRow { turns: 2, ..unknown };
        let text = cost_text(std::slice::from_ref(&mixed), &mixed, &[], OFF);
        assert!(text.contains(">=45K") && text.contains(">=189"), "{text}");
        no_esc(&text);
    }

    #[test]
    fn context_shows_the_share_and_suggests_compact_when_full() {
        let f = ContextFacts { model: "claude · s".into(), used: Some(150_000), window: Some(200_000), turns: 9, carried: 1 };
        let text = context_text(Some(&f), OFF);
        assert!(text.contains("150K of 200K tokens (75%)") && text.contains("███████████████░░░░░"), "{text}");
        assert!(text.contains("/compact") && text.contains("1 note(s)"), "{text}");
        let small = ContextFacts { used: Some(20_000), carried: 0, ..f.clone() };
        assert!(!context_text(Some(&small), OFF).contains("/compact"));
        let unknown = ContextFacts { window: None, ..f };
        assert!(context_text(Some(&unknown), OFF).contains("window size unknown"));
        assert!(context_text(None, OFF).contains("No turn"));
    }

    #[test]
    fn context_with_unknown_occupancy_has_no_share_or_compact_suggestion() {
        for window in [Some(200_000), None, Some(0)] {
            let facts = ContextFacts { model: "claude · s".into(), used: None, window, turns: 9, carried: 0 };
            let text = context_text(Some(&facts), OFF);
            assert!(text.contains("unknown (provider did not report context occupancy)"), "{text}");
            assert!(text.contains("claude · s") && text.contains("9"), "{text}");
            assert!(!text.contains('%') && !text.contains('█') && !text.contains("/compact"), "{text}");
            assert_eq!(text.contains("200K tokens"), window == Some(200_000), "{text}");
            no_esc(&text);
        }
    }

    #[test]
    fn why_gives_the_full_reason_and_the_setting() {
        let r = RouteFacts {
            provider: "codex".into(),
            model: "gpt-6-luna".into(),
            reason: "task: feature (0.9) → codex luna; warm session".into(),
            switched_provider: true,
            ..Default::default()
        };
        let text = why_text(Some(&r), "auto", true, OFF);
        assert!(text.contains("codex · gpt-6-luna") && text.contains("task: feature (0.9) → codex luna; warm session"), "{text}");
        assert!(text.contains("switched provider") && text.contains("the router picks per turn"), "{text}");
        assert!(why_text(Some(&r), "claude:claude-opus-5-5", true, OFF).contains("pinned to claude:claude-opus-5-5"));
        assert!(why_text(None, "auto", true, OFF).contains("No turn"));
    }

    #[test]
    fn config_lines_up_names_values_and_how_to_change_them() {
        let session = [ConfigRow { name: "mode".into(), value: "edit".into(), change: "/mode, Shift-Tab".into() }];
        let env = [ConfigRow { name: "JEV_API_KEY".into(), value: "set".into(), change: String::new() }];
        let text = config_text(&session, &env, &[("history".into(), "/s/history".into())], OFF);
        assert!(text.contains("  mode         edit  /mode, Shift-Tab"), "{text}");
        assert!(text.contains("  JEV_API_KEY  set\n"), "{text}");
        assert!(text.ends_with("  history  /s/history"), "{text}");
    }

    #[test]
    fn terminal_setup_shows_the_snippet_and_how_to_install() {
        let f = TerminalSetupFacts { terminal: Some("kitty".into()), file: "~/.config/kitty/kitty.conf".into(), snippet: "map a b".into(), reload: "reload".into(), can_install: true };
        let text = terminal_setup_text(&f, OFF);
        assert!(text.contains("kitty · add to ~/.config/kitty/kitty.conf") && text.contains("    map a b") && text.contains("/terminal-setup install"), "{text}");
        let unknown = TerminalSetupFacts { terminal: None, ..f };
        assert!(terminal_setup_text(&unknown, OFF).contains("could not be told"));
    }

    #[test]
    fn mcp_text_renders_empty_and_populated_tables() {
        let empty = mcp_text(&[], OFF);
        assert!(empty.contains("No MCP servers configured"), "{empty}");

        let rows = [jev_backend::infrastructure::mcp_discovery::McpRow {
            name: "test-server".into(),
            scope: jev_backend::infrastructure::mcp_discovery::McpScope::Claude,
            transport: jev_backend::infrastructure::mcp_discovery::McpTransport::Http,
            target: "https://example.com/mcp".into(),
        }];
        let populated = mcp_text(&rows, OFF);
        assert!(populated.contains("test-server"), "{populated}");
        assert!(populated.contains("claude"), "{populated}");
        assert!(populated.contains("http"), "{populated}");
        assert!(populated.contains("https://example.com/mcp"), "{populated}");
        assert!(populated.contains("/mcp login"), "{populated}");
    }
}
