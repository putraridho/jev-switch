//! Commands that read or change the setup: `/providers`, `/doctor`, `/permissions`,
//! `/workflows`.

use crate::doctor::domain::diagnose as doctor;
use crate::approval::domain::rule_policy as permissions;
use crate::approval::domain::rule_policy::PermissionsCmd;
use crate::shared::infrastructure::jsonl_log as log;
use crate::tui::render as uifmt;
use crate::tui::repl::session::Repl;
use crate::tui::views::status_facts as views;

impl Repl {
    pub(super) async fn providers(&mut self, refresh: bool) {
        let style = self.out_style();
        if refresh {
            self.console.out(&style.dim("  checking providers..."));
            self.engine.refresh_providers().await;
        }
        let rows = views::avail_rows(&self.engine.runtime, self.now());
        let mut text = uifmt::providers_table(&rows, style, self.console.width());
        for note in &self.engine.runtime.overrides.notes {
            text.push_str(&format!("\n  {}", style.dim(&format!("override: {note}"))));
        }
        self.page_out(&text);
    }

    /// The doctor's screen for this session.
    pub async fn doctor_text(&self) -> String {
        let inputs = doctor::gather(&self.engine.runtime, self.engine.dir()).await;
        views::doctor_text(&doctor::diagnose(&inputs), self.out_style(), self.console.width())
    }

    /// `/permissions`: the rules of this project, and changing them. Claude Code reads the
    /// file itself, so a change counts from the next tool call.
    pub(super) fn permissions(&mut self, cmd: PermissionsCmd) {
        let style = self.out_style();
        let dir = self.engine.dir().to_path_buf();
        let home = self.home.as_deref().map(std::path::Path::new);
        let saved = |what: &str| format!("{} {what} ({})", style.green("✓"), permissions::LOCAL_SETTINGS);
        let outcome: Result<Option<String>, String> = match cmd {
            PermissionsCmd::List => Ok(None),
            PermissionsCmd::Allow(rule) => permissions::add_allow_rule(&dir, &rule)
                .map(|added| Some(if added { saved(&format!("allowed {rule}")) } else { format!("  {}", style.dim(&format!("{rule} is already allowed"))) }))
                .map_err(|e| e.to_string()),
            PermissionsCmd::Remove(rule) => permissions::remove_allow_rule(&dir, &rule)
                .map(|removed| Some(if removed { saved(&format!("removed {rule}")) } else { format!("  {}", style.dim(&format!("{rule} is not in the project's rules (see /permissions)"))) }))
                .map_err(|e| e.to_string()),
            PermissionsCmd::Dir(arg) => permissions::resolve_dir(&arg, &dir, home).and_then(|path| {
                permissions::add_directory(&dir, &path)
                    .map(|added| Some(if added { saved(&format!("added folder {}", path.display())) } else { format!("  {}", style.dim(&format!("{} is already there", path.display()))) }))
                    .map_err(|e| e.to_string())
            }),
        };
        match outcome {
            Ok(Some(line)) => self.console.out(&line),
            Ok(None) => self.page_out(&uifmt::permissions_text(&permissions::listing(&dir, home), style, &dir)),
            Err(message) => self.console.out(&format!("{} {message}", style.yellow("⚠"))),
        }
    }

    /// `/workflows`: orchestration in this project, from the worker registry and the log.
    pub(super) fn workflows_text(&self) -> String {
        let now = self.now();
        let dir = self.engine.dir().to_path_buf();
        let open = self
            .state_dir
            .as_deref()
            .map(|s| crate::orchestration::application::delegator::open_workers(s, &dir))
            .unwrap_or_default()
            .into_iter()
            .map(|w| uifmt::OpenWorkerRow { id: w.worker_id, provider: w.provider, model: w.model, age_ms: now.saturating_sub(w.created_ms), running: w.running, shown: w.shown, merged: w.merged })
            .collect();
        let log = log::path_from_env("JEV_DELEGATE_LOG", "orchestration.jsonl");
        let rows = log.map(|p| crate::orchestration::application::delegator::recent_delegations(std::path::Path::new(&p), &dir, 200)).unwrap_or_default();
        let day_ago = now.saturating_sub(24 * 60 * 60 * 1000);
        let today: Vec<_> = rows.iter().filter(|d| d.ts_ms >= day_ago).collect();
        let day = (today.len(), today.iter().filter(|d| d.ok).count(), today.iter().map(|d| d.wall_ms).sum());
        let skip = rows.len().saturating_sub(10);
        let recent = rows
            .into_iter()
            .skip(skip)
            .map(|d| uifmt::DelegationRow {
                ago: views::human_ago(now.saturating_sub(d.ts_ms)),
                id: d.worker_id.unwrap_or_else(|| "-".into()),
                task: d.task.unwrap_or_else(|| "-".into()),
                provider: d.provider,
                model: d.model,
                ok: d.ok,
                error_kind: d.error_kind,
                wall_ms: d.wall_ms,
                files: d.files_changed.len(),
                tools: d.tool_calls.len(),
                attempt: d.attempt.unwrap_or(1),
            })
            .collect();
        let facts = uifmt::WorkflowFacts { orchestrate: self.engine.orchestrate(), open, recent, day };
        uifmt::workflows_text(&facts, self.out_style(), self.console.width())
    }
}
