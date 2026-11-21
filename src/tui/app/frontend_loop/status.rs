//! What the screen shows about the session: repository line, provider and worker chips, route.

use crate::shared::infrastructure::jsonl_log as log;
use crate::tui::app::screen_state::App;
use crate::tui::repl::session::Repl;
use crate::tui::views::status_facts as views;

fn git_command(dir: &std::path::Path, args: &[&str]) -> std::process::Command {
    let mut command = std::process::Command::new("git");
    command.args(crate::shared::infrastructure::git_guard::no_repo_programs(dir)).args(args).current_dir(dir).stderr(std::process::Stdio::null());
    command
}

/// The branch and how many files differ from it, `main ±3`; `None` outside a repository.
fn git_line(dir: &std::path::Path) -> Option<String> {
    let git = |args: &[&str]| {
        git_command(dir, args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    let changed = git(&["status", "--porcelain"]).map_or(0, |s| s.lines().count());
    Some(if changed > 0 { format!("{branch} ±{changed}") } else { branch })
}

#[cfg(test)]
mod tests;

/// What the router row shows: each subscription that can take a turn, with how much of its
/// plan window is used, and those cooling down after a limit.
pub(super) fn provider_chips(repl: &Repl) -> Vec<crate::tui::app::screen_state::ProviderChip> {
    let rt = &repl.engine.runtime;
    views::providers::avail_rows(rt, log::now_ms())
        .into_iter()
        .filter(|r| r.status == crate::tui::render::ProviderStatus::Ready)
        .map(|r| {
            let used = r.provider.parse::<crate::backend::ProviderId>().ok().and_then(|p| rt.plan_utilization(p)).map(|u| (u.clamp(0.0, 1.0) * 100.0).round() as u8);
            let credit = r.provider.parse::<crate::backend::ProviderId>().ok().and_then(|p| rt.credit_left(p));
            crate::tui::app::screen_state::ProviderChip { name: r.provider, used, cooling: r.cooldown, via: r.via, credit }
        })
        .collect()
}

/// Bring the screen's view of the session up to date (between lines).
pub(super) fn refresh(app: &mut App, repl: &Repl) {
    app.route = route_label(repl);
    app.ctx = views::providers::complete_ctx(&repl.engine, log::now_ms());
    app.git = git_line(repl.engine.dir());
    app.context = repl.context_percent();
    app.session_name = repl.engine.sessions.name().map(String::from);
    app.providers = provider_chips(repl);
    app.orchestrate = repl.engine.orchestrate();
}

/// The project's open workers, for the rows above the box.
pub(super) fn worker_chips(state_dir: Option<&std::path::Path>, project: &std::path::Path) -> Vec<crate::tui::app::screen_state::WorkerChip> {
    let now = log::now_ms();
    state_dir
        .map(|s| crate::orchestration::infrastructure::worker_file::open_workers(s, project))
        .unwrap_or_default()
        .into_iter()
        .map(|w| crate::tui::app::screen_state::WorkerChip { id: w.worker_id, provider: w.provider, model: w.model, running: w.running, age_ms: now.saturating_sub(w.created_ms), tools: w.tools, activity: w.activity })
        .collect()
}

/// The footer's routing word: the pin, `claude only`, or `auto`.
pub(super) fn route_label(repl: &Repl) -> String {
    views::prompt::prompt_label(repl.engine.pin(), repl.engine.enabled()).unwrap_or_else(|| "auto".to_string())
}
