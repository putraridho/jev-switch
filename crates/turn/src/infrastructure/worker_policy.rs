//! The [`WorkerPolicy`] of a delegated worker: the same privacy policy, secret scan and audit
//! log the planner's own turns answer to ([`Guards`]), the permission mode and project trust
//! a worker's turn runs under, and the rule that answers its tool calls.

use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use jev_approval::application::extension_hooks::trust::project_settings_trusted;
use jev_approval::domain::mode::Mode;
use jev_approval::domain::permission_rule;
use jev_approval::domain::worker_command::worker_may_run;
use jev_backend::{Approver, Decision, PermissionRequest, ProviderId, TurnRequest, NO_ONE_TO_ASK};
use jev_orchestration::application::ports::{Screen, WorkerPolicy};
use crate::application::guards::Guards;
use crate::domain::privacy::outgoing;

/// Wraps the user's [`Guards`] (from `Guards::load`; `Guards::default()` for the default
/// secret scan only).
pub struct GuardedWorkers(pub Guards);

impl WorkerPolicy for GuardedWorkers {
    fn screen(&self, repo: &Path, outgoing_text: &str, brief: &str) -> Screen {
        let verdict = self.0.privacy.check(repo, outgoing_text, &[], self.0.home.as_deref());
        let permitted = jev_shared::ALL_PROVIDERS.into_iter().filter(|p| verdict.permits(*p)).collect();
        let judge_prompt = self.0.may_leave(repo, outgoing_text, &[]).then(|| outgoing::prepare(&self.0.privacy, brief).prompt);
        Screen { permitted, refusal: verdict.refusal_line(), judge_prompt }
    }

    fn outgoing(&self, prompt: &str, provider: ProviderId, model: &str, now_ms: u64) -> String {
        self.0.outgoing(prompt, provider, model, now_ms).prompt
    }

    fn request(&self, dir: &Path, model: &str, prompt: String, read_only: bool) -> TurnRequest {
        let mode = if read_only { Mode::Plan } else { Mode::default() };
        TurnRequest { dir: dir.to_path_buf(), model: model.to_string(), prompt, images: vec![], session_id: None, mode, trust_project_settings: project_settings_trusted(dir) }
    }

    fn approver(&self, dir: &Path, read_only: bool) -> Arc<dyn Approver> {
        let allowed_tools = permission_rule::split_rules(&jev_shared::infrastructure::env_file::lookup("CODE_ALLOWED_TOOLS").unwrap_or_default());
        Arc::new(WorkerApprover { allowed_tools, read_only, dir: dir.to_path_buf() })
    }
}

/// Answers a worker's tool calls with nobody to ask (a CLI that asks jev-code: Gemini on
/// Antigravity, Grok over ACP; Claude Code and Codex decide inside their own CLI). Allowed:
/// what `CODE_ALLOWED_TOOLS` allows, as Claude Code workers are told with `--allowedTools`
/// (not in plan mode, as in the planner's own turns); a shell command [`worker_may_run`] judges
/// safe in the worker's own folder; and for an editing worker an `Edit` of a file inside its
/// folder (Grok has no accept-edits mode, so it asks about every edit). Everything else is
/// denied as having nobody to ask, so the failure still names `CODE_ALLOWED_TOOLS`.
pub struct WorkerApprover {
    pub allowed_tools: Vec<String>,
    pub read_only: bool,
    /// The worker's own folder: its git worktree.
    pub dir: PathBuf,
}

impl WorkerApprover {
    pub fn decision(&self, req: &PermissionRequest) -> Decision {
        let command = req.input.str_field("command");
        let by_setting = !self.read_only && permission_rule::allowed_by(&self.allowed_tools, &req.tool, command);
        let safe_here = req.tool == "Bash" && req.blocked_path.is_none() && command.is_some_and(worker_may_run);
        let edit_here = !self.read_only && req.tool == "Edit" && req.input.str_field("file_path").is_some_and(|f| self.inside(f));
        if by_setting || safe_here || edit_here {
            Decision::Allow
        } else {
            Decision::Deny { reason: NO_ONE_TO_ASK.to_string() }
        }
    }

    /// Whether `file` (absolute, or relative to the worker's folder) is inside that folder
    /// and not in its `.git`, judged on the words of the path (no `..`; a `.` is dropped).
    fn inside(&self, file: &str) -> bool {
        let path = Path::new(file);
        let full = if path.is_absolute() { path.to_path_buf() } else { self.dir.join(path) };
        let Ok(rest) = full.strip_prefix(&self.dir) else { return false };
        let mut parts = rest.components().peekable();
        parts.peek().is_some()
            && rest.components().all(|c| matches!(c, Component::Normal(_)))
            && !matches!(parts.next(), Some(Component::Normal(first)) if first == ".git")
    }
}

impl Approver for WorkerApprover {
    fn decide<'a>(&'a self, req: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        let decision = self.decision(req);
        Box::pin(async move { decision })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_approval::infrastructure::tool_input_wire::tool_input_from_json;

    fn bash(command: &str) -> PermissionRequest {
        let input = tool_input_from_json(&serde_json::json!({ "command": command }));
        PermissionRequest { id: "1".into(), tool: "Bash".into(), summary: command.into(), input, blocked_path: None }
    }

    fn worker(allowed: &[&str], read_only: bool) -> WorkerApprover {
        WorkerApprover { allowed_tools: allowed.iter().map(|s| s.to_string()).collect(), read_only, dir: PathBuf::from("/state/worktrees/w1") }
    }

    fn edit(file: &str) -> PermissionRequest {
        let input = tool_input_from_json(&serde_json::json!({ "file_path": file }));
        PermissionRequest { id: "2".into(), tool: "Edit".into(), summary: file.into(), input, blocked_path: None }
    }

    #[test]
    fn an_editing_worker_edits_inside_its_worktree_only() {
        let w = worker(&[], false);
        for inside in ["/state/worktrees/w1/src/a.rs", "src/a.rs", "README.md", "./src/a.rs"] {
            assert_eq!(w.decision(&edit(inside)), Decision::Allow, "{inside}");
        }
        for outside in [
            "/state/worktrees/w1",
            "/state/worktrees/w1/../w2/a.rs",
            "../x.rs",
            "src/../../x.rs",
            "/home/u/.bashrc",
            "/state/worktrees/w10/a.rs",
            "/state/worktrees/w1/.git/config",
            ".git/hooks/pre-commit",
        ] {
            assert!(matches!(w.decision(&edit(outside)), Decision::Deny { .. }), "{outside}");
        }
    }

    #[test]
    fn a_read_only_worker_never_edits() {
        assert!(matches!(worker(&[], true).decision(&edit("src/a.rs")), Decision::Deny { .. }));
    }

    #[test]
    fn safe_commands_run_and_the_rest_are_denied_as_nobody_to_ask() {
        let w = worker(&[], true);
        assert_eq!(w.decision(&bash("git diff")), Decision::Allow);
        assert_eq!(w.decision(&bash("grep -rn foo src")), Decision::Allow);
        for denied in ["rm -rf src", "cat ~/.ssh/id_rsa", "git commit -m x", "python3 x.py"] {
            assert_eq!(w.decision(&bash(denied)), Decision::Deny { reason: NO_ONE_TO_ASK.into() }, "{denied}");
        }
    }

    #[test]
    fn code_allowed_tools_apply_to_editing_workers_only() {
        assert_eq!(worker(&["Bash(python3 *)"], false).decision(&bash("python3 x.py")), Decision::Allow);
        assert!(matches!(worker(&["Bash(python3 *)"], true).decision(&bash("python3 x.py")), Decision::Deny { .. }));
    }

    #[test]
    fn a_command_blocked_by_its_path_or_another_tool_is_not_auto_allowed() {
        let w = worker(&[], false);
        let mut outside = bash("cat notes.txt");
        outside.blocked_path = Some("/etc".into());
        assert!(matches!(w.decision(&outside), Decision::Deny { .. }));
        let mut other = bash("git diff");
        other.tool = "acp_WebFetch".into();
        assert!(matches!(w.decision(&other), Decision::Deny { .. }));
    }
}
