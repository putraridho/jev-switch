//! The [`WorkerPolicy`] of a delegated worker: the same privacy policy, secret scan and audit
//! log the planner's own turns answer to ([`Guards`]), the permission mode and project trust
//! a worker's turn runs under, and the rule that answers its tool calls.

use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

use jev_approval::application::extension_hooks::trust::project_settings_trusted;
use jev_approval::domain::mode::Mode;
use jev_approval::domain::permission_rule;
use jev_approval::domain::worker_command::worker_may_run;
use jev_backend::{Approver, Decision, PermissionRequest, ProviderId, TurnRequest, Usage, NO_ONE_TO_ASK};
use jev_orchestration::application::ports::{BudgetHold, Screen, WorkerPolicy};
use crate::application::guards::Guards;
use crate::domain::privacy::outgoing;

/// Owns the worker guards: admits each worker attempt against the same budget as the planner's
/// turns (reserving its estimated maximum in the shared usage ledger) and settles it with what
/// it used. The ledger is how this process sees the planner's spend and the planner sees its.
pub struct GuardedWorkers {
    guards: Mutex<Guards>,
}

impl GuardedWorkers {
    /// Wraps the user's [`Guards`] with synchronized worker admission and usage recording.
    pub fn new(guards: Guards) -> Self {
        Self { guards: Mutex::new(guards) }
    }

    fn guards(&self) -> MutexGuard<'_, Guards> {
        self.guards.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl WorkerPolicy for GuardedWorkers {
    fn screen(&self, repo: &Path, outgoing_text: &str, brief: &str) -> Screen {
        let guards = self.guards();
        let verdict = guards.privacy.check(repo, outgoing_text, &[], guards.home.as_deref());
        let permitted = jev_shared::ALL_PROVIDERS.into_iter().filter(|p| verdict.permits(*p)).collect();
        let judge_prompt = guards.may_leave(repo, outgoing_text, &[]).then(|| outgoing::prepare(&guards.privacy, brief).prompt);
        Screen { permitted, refusal: verdict.refusal_line(), judge_prompt }
    }

    fn admits(&self, provider: ProviderId, model: &str, attempts: u32) -> bool {
        let guards = self.guards();
        !guards.is_paid(provider, model) || guards.may_pay(attempts)
    }

    fn hold(&self, provider: ProviderId, model: &str, prompt_chars: usize) -> Result<BudgetHold, String> {
        self.guards().hold(provider, model, (prompt_chars / 4) as u64)
    }

    fn settle(&self, hold: BudgetHold, provider: ProviderId, model: &str, usage: &Usage) {
        self.guards().settle(&hold, provider, model, usage);
    }

    fn release(&self, hold: BudgetHold) {
        self.guards().release(&hold);
    }

    fn record_usage(&self, provider: ProviderId, model: &str, usage: &Usage) {
        self.guards().record_spend(provider, model, usage);
    }

    fn outgoing(&self, prompt: &str, provider: ProviderId, model: &str, now_ms: u64) -> String {
        self.guards().outgoing(prompt, provider, model, now_ms).prompt
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
    use crate::domain::budget::{Budget, Cap, PriceTable};
    use crate::application::usage_account::UsageAccount;
    use crate::domain::failover_policy::FailoverPolicy;
    use crate::infrastructure::memory_usage_ledger::{KnownProcesses, MemoryUsageLedger};

    fn capped(cap: Cap) -> Guards {
        Guards { budget: Budget { cap: Some(cap), ..Budget::default() }, ..Guards::default() }
    }

    /// A planner and a worker process sharing one ledger, as two processes share the file.
    fn sharing(ledger: &Arc<MemoryUsageLedger>, cap: Cap, planner: bool, pid: u32) -> Guards {
        let live = Arc::new(KnownProcesses::new(pid));
        let path = PathBuf::from("ledger");
        let account = if planner { UsageAccount::planner(ledger.clone(), live, path, Path::new("/repo"), 1_000) } else { UsageAccount::worker(ledger.clone(), live, path, Path::new("/repo"), 1_000) };
        let account = account.with_clock(|| 1_000);
        capped(cap).with_ledger(account)
    }

    const FLASH: (ProviderId, &str) = (ProviderId::DeepSeek, "deepseek-flash");

    #[test]
    fn a_workers_spend_counts_against_the_planners_cap_and_the_planners_against_the_workers() {
        let ledger = Arc::new(MemoryUsageLedger::default());
        let mut planner = sharing(&ledger, Cap::Tokens(100), true, 1);
        let worker = GuardedWorkers::new(sharing(&ledger, Cap::Tokens(100), false, 2));
        assert!(planner.may_pay(0) && worker.admits(FLASH.0, FLASH.1, 0));

        worker.record_usage(FLASH.0, FLASH.1, &Usage { input_tokens: 60, ..Usage::default() });
        assert_eq!(planner.meter.paid().1, 0, "the planner's own meter is only its own");
        assert_eq!(planner.total_meter().paid().1, 60);
        assert!(planner.may_pay(0), "60 of 100");

        planner.record_spend(FLASH.0, FLASH.1, &Usage { input_tokens: 40, ..Usage::default() });
        assert!(!planner.may_pay(0), "the worker's 60 and the planner's 40 reach the cap");
        assert!(!worker.admits(FLASH.0, FLASH.1, 0), "and the worker sees the planner's spend");
        assert_eq!(planner.total_meter().paid().1, 100, "nothing is counted twice");
    }

    #[test]
    fn a_worker_attempt_and_a_planner_attempt_reserve_against_the_same_cap() {
        let ledger = Arc::new(MemoryUsageLedger::default());
        // 20k per attempt (4k prompt + the 16k output allowance): two fit 50k, a third does not.
        let planner = sharing(&ledger, Cap::Tokens(50_000), true, 1);
        let worker = GuardedWorkers::new(sharing(&ledger, Cap::Tokens(50_000), false, 2));
        let main = planner.hold(FLASH.0, FLASH.1, 4_000).unwrap();
        let first = worker.hold(FLASH.0, FLASH.1, 16_000).unwrap();
        assert!(first.id.is_some());
        let refused = worker.hold(FLASH.0, FLASH.1, 16_000).unwrap_err();
        assert!(refused.contains("reserved"), "{refused}");
        assert!(planner.hold(FLASH.0, FLASH.1, 4_000).is_err(), "the planner sees the worker's reservation too");

        // The worker finishes with less than it reserved; the freed room is usable again.
        worker.settle(first, FLASH.0, FLASH.1, &Usage { input_tokens: 100, ..Usage::default() });
        assert!(planner.hold(FLASH.0, FLASH.1, 4_000).is_ok());
        let mut planner = planner;
        planner.settle(&main, FLASH.0, FLASH.1, &Usage { input_tokens: 50, ..Usage::default() });
        assert_eq!(planner.total_meter().paid().1, 150);
    }

    #[test]
    fn a_cancelled_or_failed_worker_without_a_report_settles_as_unknown_and_closes_the_cap() {
        let ledger = Arc::new(MemoryUsageLedger::default());
        let planner = sharing(&ledger, Cap::Tokens(1_000_000), true, 1);
        let worker = GuardedWorkers::new(sharing(&ledger, Cap::Tokens(1_000_000), false, 2));
        let hold = worker.hold(FLASH.0, FLASH.1, 100).unwrap();
        worker.settle(hold, FLASH.0, FLASH.1, &Usage { measurement: jev_shared::Measurement::Unavailable, ..Usage::default() });
        assert!(!planner.may_pay(0));
        assert!(planner.hold(FLASH.0, FLASH.1, 1).is_err());
    }

    #[test]
    fn a_released_hold_frees_its_reservation_without_spend() {
        let ledger = Arc::new(MemoryUsageLedger::default());
        let worker = GuardedWorkers::new(sharing(&ledger, Cap::Tokens(25_000), true, 1));
        let hold = worker.hold(FLASH.0, FLASH.1, 100).unwrap();
        assert!(worker.hold(FLASH.0, FLASH.1, 100).is_err());
        worker.release(hold);
        assert!(worker.hold(FLASH.0, FLASH.1, 100).is_ok());
        assert_eq!(worker.guards().total_meter().paid().1, 0);
    }

    #[test]
    fn a_new_planner_session_starts_fresh_for_the_workers_that_join_it() {
        let ledger = Arc::new(MemoryUsageLedger::default());
        let mut old = sharing(&ledger, Cap::Tokens(100), true, 1);
        old.record_spend(FLASH.0, FLASH.1, &Usage { input_tokens: 500, ..Usage::default() });
        let planner = {
            let live = Arc::new(KnownProcesses::new(9));
            capped(Cap::Tokens(100)).with_ledger(UsageAccount::planner(ledger.clone(), live, PathBuf::from("ledger"), Path::new("/repo"), 2_000).with_clock(|| 2_000))
        };
        assert!(planner.may_pay(0), "the earlier session's 500 is not this session's");
        let worker = GuardedWorkers::new(sharing(&ledger, Cap::Tokens(100), false, 2));
        assert!(worker.admits(FLASH.0, FLASH.1, 0), "a worker joins the newest session of its project");
    }

    #[test]
    fn unreported_worker_spend_makes_a_capped_planner_fail_closed() {
        let ledger = Arc::new(MemoryUsageLedger::default());
        let planner = sharing(&ledger, Cap::Tokens(1_000_000), true, 1);
        let worker = GuardedWorkers::new(sharing(&ledger, Cap::Tokens(1_000_000), false, 2));
        worker.record_usage(FLASH.0, FLASH.1, &Usage { measurement: jev_shared::Measurement::Unavailable, ..Usage::default() });
        assert!(!planner.may_pay(0));
    }

    #[test]
    fn reached_paid_cap_rejects_deepseek_without_blocking_claude_subscription() {
        let mut guards = capped(Cap::Tokens(100));
        guards.meter.record(ProviderId::DeepSeek, "deepseek-v4-pro", &Usage { input_tokens: 100, ..Usage::default() }, &guards.prices);
        let policy = GuardedWorkers::new(guards);
        assert!(policy.screen(Path::new("/repo"), "fix a bug", "fix a bug").permitted.contains(&ProviderId::DeepSeek));
        assert!(!policy.admits(ProviderId::DeepSeek, "deepseek-flash", 0));
        assert!(!policy.admits(ProviderId::Claude, "claude-fable-5-2", 0));
        assert!(policy.admits(ProviderId::Claude, "claude-sonnet-5-5", 0));
    }

    #[test]
    fn paid_fallback_off_allows_first_attempt_but_not_retries() {
        let policy = GuardedWorkers::new(Guards {
            failover: FailoverPolicy { paid_fallback: false, ..FailoverPolicy::default() },
            ..Guards::default()
        });
        for (provider, model) in [(ProviderId::DeepSeek, "deepseek-flash"), (ProviderId::Claude, "claude-fable-5-2")] {
            assert!(policy.admits(provider, model, 0));
            assert!(!policy.admits(provider, model, 1));
            assert!(!policy.admits(provider, model, 2));
        }
        assert!(policy.admits(ProviderId::Claude, "claude-sonnet-5-5", 1));
    }

    #[test]
    fn a_worker_on_a_custom_codex_backend_needs_the_paid_fallback() {
        use jev_providers::domain::funding::{Funding, Fundings};
        let mut guards = Guards { failover: FailoverPolicy { max_attempts: 6, paid_fallback: false }, ..Guards::default() };
        let native = GuardedWorkers::new(Guards { failover: guards.failover, ..Guards::default() });
        assert!(native.admits(ProviderId::Codex, "gpt-6.1-terra", 1));
        guards.set_fundings(Fundings::default().with(ProviderId::Codex, Funding::Custom));
        let policy = GuardedWorkers::new(guards);
        assert!(policy.admits(ProviderId::Codex, "gpt-6.1-terra", 0));
        assert!(!policy.admits(ProviderId::Codex, "gpt-6.1-terra", 1));
    }

    #[test]
    fn paid_fallback_on_does_not_bypass_the_budget() {
        let policy = GuardedWorkers::new(capped(Cap::Tokens(100)));
        policy.record_usage(ProviderId::DeepSeek, "deepseek-flash", &Usage { output_tokens: 100, ..Usage::default() });
        assert!(!policy.admits(ProviderId::DeepSeek, "deepseek-flash", 0));
        assert!(!policy.admits(ProviderId::DeepSeek, "deepseek-flash", 1));
        assert!(GuardedWorkers::new(Guards::default()).admits(ProviderId::DeepSeek, "deepseek-flash", 1));
    }

    #[test]
    fn completed_worker_usage_is_seen_by_subsequent_admission() {
        let policy = GuardedWorkers::new(capped(Cap::Tokens(100)));
        assert!(policy.admits(ProviderId::DeepSeek, "deepseek-flash", 0));
        policy.record_usage(ProviderId::DeepSeek, "deepseek-flash", &Usage { input_tokens: 60, output_tokens: 40, ..Usage::default() });
        assert!(!policy.admits(ProviderId::DeepSeek, "deepseek-flash", 0));
        assert!(policy.admits(ProviderId::Claude, "claude-sonnet-5-5", 0));
    }

    #[test]
    fn concurrent_completions_accumulate_in_the_same_meter() {
        let policy = GuardedWorkers::new(capped(Cap::Tokens(100)));
        std::thread::scope(|scope| {
            for _ in 0..10 {
                scope.spawn(|| policy.record_usage(ProviderId::DeepSeek, "deepseek-flash", &Usage { output_tokens: 10, ..Usage::default() }));
            }
        });
        assert_eq!(policy.guards().meter.paid().1, 100);
        assert!(!policy.admits(ProviderId::DeepSeek, "deepseek-flash", 0));
    }

    #[test]
    fn unknown_paid_usd_spend_fails_closed_like_main_guards() {
        let policy = GuardedWorkers::new(capped(Cap::Usd(1.0)));
        policy.record_usage(ProviderId::DeepSeek, "unknown-model", &Usage { output_tokens: 10, ..Usage::default() });
        assert!(!policy.admits(ProviderId::DeepSeek, "deepseek-flash", 0));
        assert!(!policy.admits(ProviderId::Claude, "claude-fable-5-2", 0));
        assert!(policy.admits(ProviderId::Claude, "claude-sonnet-5-5", 0));
    }

    #[test]
    fn priced_paid_usage_reaches_usd_cap_but_subscription_usage_does_not() {
        let mut guards = capped(Cap::Usd(1.0));
        guards.prices = PriceTable::parse("deepseek-flash 1 0 1").0;
        let policy = GuardedWorkers::new(guards);
        policy.record_usage(ProviderId::Claude, "claude-sonnet-5-5", &Usage { input_tokens: 1_000_000, ..Usage::default() });
        assert!(policy.admits(ProviderId::DeepSeek, "deepseek-flash", 0));
        policy.record_usage(ProviderId::DeepSeek, "deepseek-flash", &Usage { input_tokens: 1_000_000, ..Usage::default() });
        assert!(!policy.admits(ProviderId::DeepSeek, "deepseek-flash", 0));
    }

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
