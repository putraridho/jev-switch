//! The presentation layer of `jev-code`: every string the terminal UI prints is built
//! here, as a pure function from plain facts to a `String`, so the binary only decides
//! WHERE to print and each wording, colour and width rule is a unit test.
//!
//! Conventions, the same for every formatter:
//! - Input structs hold plain strings and numbers, never engine types, so the engine
//!   can change without touching the look.
//! - A [`Style`] that is disabled produces zero escape bytes. Colour is an addition to
//!   a line that already reads fine without it.
//! - Results have no trailing newline (except [`answer_text`], which keeps the line
//!   structure of the text it was given).
//! - Formatters that take a `width` clamp it to at least [`MIN_WIDTH`] columns (a width
//!   of 0 usually means "unknown"); the raw helpers ([`truncate_to_width`],
//!   [`wrap_to_width`], [`status_line`]) honour the width exactly.
//! - Widths are terminal columns, from the `unicode-width` crate: wide characters count
//!   2, combining marks 0, and ANSI escape sequences 0.
//!
//! The module is split by concern (`ansi`, `width`, `wrap`, `text`, `style` for the
//! plumbing; `route`, `failure`, `summary`, `changes`, `provider_view`, `screens`,
//! `markdown`, `status`, `complete` for what is shown; `commands` for the slash-command
//! table), and this file re-exports the whole public API, so callers only ever name
//! `uifmt::item`.

mod ansi;
mod changes;
mod commands;
mod complete;
mod diffview;
mod failure;
mod highlight;
mod identity;
mod markdown;
mod permissions;
mod provider_view;
mod roles;
mod route;
mod screens;
mod session;
mod status;
mod style;
mod summary;
mod text;
mod width;
mod workflows;
mod wrap;

#[cfg(test)]
mod testutil;

pub use ansi::{clean_text, clear_line, hide_cursor, restore_cursor, save_cursor, show_cursor, strip_ansi, up_row};
pub use changes::{changes_block, diff_colorize, relativize_paths, tool_line, FileChange, MAX_CHANGE_ROWS};
pub use commands::{command_names, command_usage, Cmd, Group, COMMANDS, KEYS};
pub use complete::{complete, completion_start, hint, Candidate, CompleteCtx, SkillChoice};
pub use diffview::{render_diff, render_hunk, word_diff, DiffOpts, Fold, Layout, Piece, FOLD_AFTER, SPLIT_MIN_WIDTH};
pub use failure::{error_block, failover_note, next_steps_for, no_route_message, problem_block};
pub use identity::{builtin_identity, ChipFacts, ColorDepth, Health, Identity, NativeEvent, OverrideError, Palette, ProviderTheme, Rgb, ThemeOverrides};
pub use markdown::answer_text;
pub use permissions::{auto_allowed, denied_without_asking, remembered_allowed, permission_answered, permission_prompt, permissions_text, Prompt, Question};
pub use provider_view::{provider_glyphs, providers_table, AvailRow, ProviderStatus};
pub use roles::{answer_block, answer_panel, display_tool, edit_preview, edit_preview_at, narration, route_row, sent_block, thinking_block, tool_name, tool_output_fold, tool_result_row, tool_row, tool_run_summary, EDIT_PREVIEW_LINES, FOLD_KEEP, THINKING_LINES};
pub use route::{route_line, short_reason, RouteFacts};
pub use screens::{doctor_report, doctor_summary, help_text, status_text, DoctorItem, ReadyProvider, Severity, StatusFacts};
pub use session::{config_text, context_text, cost_text, mcp_text, terminal_setup_text, why_text, ConfigRow, ContextFacts, CostRow, TerminalSetupFacts, COMPACT_SUGGEST_AT};
pub use status::{input_row, status_line, Spinner, SPINNER_FRAMES, SPINNER_FRAME_MS};
pub use style::{background_from_env, light_background, set_light_background, stderr_is_tty, stdout_is_tty, Style, Theme};
pub use summary::{turn_rule, turn_summary, turn_summary_brief, SummaryFacts};
pub use text::{human_count, human_duration};
pub use width::{char_width, display_width, terminal_width, truncate_to_width, width_from_env, DEFAULT_WIDTH, MIN_WIDTH};
pub use wrap::wrap_to_width;
pub use workflows::{workflows_text, DelegationRow, OpenWorkerRow, WorkflowFacts};

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use crate::backend::ErrorKind;

    use super::*;
    use crate::uifmt::testutil::*;

    #[test]
    fn a_disabled_style_never_emits_escapes() {
        let all_rows = rows();
        let kinds = all_kinds();
        for kind in &kinds {
            no_esc(&failover_note("claude", "opus", kind, Some(("codex", "g")), true, OFF));
            no_esc(&error_block(kind, "message", &next_steps_for(kind, "claude"), OFF));
        }
        no_esc(&route_line(&RouteFacts { provider: "codex".into(), model: "m".into(), reason: "task: a (0.2) → b; first turn".into(), ..Default::default() }, OFF));
        no_esc(&no_route_message("security", &all_rows, OFF));
        no_esc(&turn_summary(&facts(), OFF));
        no_esc(&changes_block(&[fc('M', "a", 1, 1)], OFF, 40));
        no_esc(&diff_colorize("+a\n-b\n@@ x\n", OFF));
        no_esc(&tool_line("Edit a", Path::new("/p"), OFF, 40));
        no_esc(&help_text(OFF, 100));
        no_esc(&status_text(&StatusFacts::default(), OFF));
        no_esc(&providers_table(&all_rows, OFF, 80));
        no_esc(&doctor_report(&[item(Severity::Fail, "t", "d", &["c"])], OFF, 80));
        no_esc(&doctor_summary(&[item(Severity::Fail, "t", "d", &["c"])], OFF));
        no_esc(&status_line("m", Duration::from_secs(1), Some("a"), 40, OFF));
    }

    #[test]
    fn styling_only_adds_escapes_never_words() {
        let all_rows = rows();
        let pairs: Vec<(String, String)> = vec![
            (
                no_route_message("security", &all_rows, ON),
                no_route_message("security", &all_rows, OFF),
            ),
            (
                error_block(&ErrorKind::Auth, "bad\nthing", &["Do `x`".to_string()], ON),
                error_block(&ErrorKind::Auth, "bad\nthing", &["Do `x`".to_string()], OFF),
            ),
            (changes_block(&[fc('M', "a", 1, 1)], ON, 40), changes_block(&[fc('M', "a", 1, 1)], OFF, 40)),
            (tool_line("Edit a", Path::new("/p"), ON, 40), tool_line("Edit a", Path::new("/p"), OFF, 40)),
            (help_text(ON, 100), help_text(OFF, 100)),
            (doctor_summary(&[item(Severity::Warn, "t", "", &[])], ON), doctor_summary(&[item(Severity::Warn, "t", "", &[])], OFF)),
            (status_line("m", Duration::from_secs(3), Some("a"), 40, ON), status_line("m", Duration::from_secs(3), Some("a"), 40, OFF)),
        ];
        for (styled, plain) in pairs {
            assert_eq!(strip_ansi(&styled), plain);
        }
    }
}
