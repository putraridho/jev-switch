use super::*;
use jev_backend::Measurement;
use crate::application::ports::BudgetHold;

#[derive(Default)]
struct MeteredPolicy {
    denied: Vec<(ProviderId, &'static str)>,
    paid_fallback_off: bool,
    cap: Option<u64>,
    /// Models the budget refuses at the moment the attempt asks to start (a hold).
    refused: Vec<(ProviderId, &'static str)>,
    recorded: Mutex<Vec<(ProviderId, String, Usage)>>,
    held: Mutex<u32>,
}

impl WorkerPolicy for MeteredPolicy {
    fn screen(&self, repo: &Path, outgoing: &str, brief: &str) -> Screen {
        OpenPolicy.screen(repo, outgoing, brief)
    }

    fn admits(&self, provider: ProviderId, model: &str, attempts: u32) -> bool {
        if self.denied.contains(&(provider, model)) {
            return false;
        }
        if provider != DeepSeek {
            return true;
        }
        let recorded = self.recorded.lock().unwrap();
        let unknown_paid = recorded.iter().any(|(provider, _, usage)| *provider == DeepSeek && usage.measurement == Measurement::Unavailable);
        let spent: u64 = recorded.iter()
            .filter(|(provider, _, _)| *provider == DeepSeek)
            .map(|(_, _, usage)| usage.input_tokens + usage.output_tokens).sum();
        (!self.paid_fallback_off || attempts == 0) && self.cap.is_none_or(|cap| !unknown_paid && spent < cap)
    }

    fn hold(&self, provider: ProviderId, model: &str, _prompt_chars: usize) -> Result<BudgetHold, String> {
        if self.refused.iter().any(|(p, m)| *p == provider && *m == model) {
            return Err("the paid cap would be passed".into());
        }
        *self.held.lock().unwrap() += 1;
        Ok(BudgetHold { id: Some("h".into()), warning: None })
    }

    fn release(&self, _hold: BudgetHold) {
        *self.held.lock().unwrap() -= 1;
    }

    fn settle(&self, _hold: BudgetHold, provider: ProviderId, model: &str, usage: &Usage) {
        *self.held.lock().unwrap() -= 1;
        self.record_usage(provider, model, usage);
    }

    fn record_usage(&self, provider: ProviderId, model: &str, usage: &Usage) {
        self.recorded.lock().unwrap().push((provider, model.to_string(), *usage));
    }

    fn outgoing(&self, prompt: &str, provider: ProviderId, model: &str, now_ms: u64) -> String {
        OpenPolicy.outgoing(prompt, provider, model, now_ms)
    }

    fn request(&self, dir: &Path, model: &str, prompt: String, read_only: bool) -> jev_backend::TurnRequest {
        OpenPolicy.request(dir, model, prompt, read_only)
    }

    fn approver(&self, dir: &Path, read_only: bool) -> Arc<dyn jev_backend::Approver> {
        OpenPolicy.approver(dir, read_only)
    }
}

fn policy_rig(ready: &[ProviderId], policy: Arc<MeteredPolicy>) -> Rig {
    let mut rig = rig(ready, TaskType::Feature);
    *lock(&rig.d.runtime) = Runtime::for_tests(jev_shared::ALL_PROVIDERS.iter().map(|provider| availability(*provider, ready.contains(provider))).collect());
    rig.scripts.lock().unwrap().insert(DeepSeek, OK);
    rig.d.policy = policy;
    rig
}

#[tokio::test]
async fn model_admission_filters_before_worker_pick_without_removing_the_provider() {
    let policy = Arc::new(MeteredPolicy { denied: vec![(Claude, "claude-haiku-5-5")], ..MeteredPolicy::default() });
    let rig = policy_rig(&[Claude], policy.clone());
    let result = rig.d.delegate(brief("x")).await;
    assert!(result.ok, "{:?}", result.error);
    assert_eq!(result.model.as_deref(), Some("claude-sonnet-5-5"));
    assert_eq!(policy.recorded.lock().unwrap().len(), 1);

    let policy = Arc::new(MeteredPolicy { denied: vec![(Claude, "claude-sonnet-5-5")], ..MeteredPolicy::default() });
    let rig = policy_rig(&[Claude, Codex], policy);
    assert_eq!(rig.d.delegate(brief("x")).await.provider, Some(Codex));
}

#[tokio::test]
async fn reached_cap_stops_a_paid_worker_before_worktree_or_cli_creation() {
    let rig = policy_rig(&[DeepSeek], Arc::new(MeteredPolicy { cap: Some(0), ..MeteredPolicy::default() }));
    let result = rig.d.delegate(brief("x")).await;
    assert!(!result.ok);
    assert!(rig.started.lock().unwrap().is_empty());
    assert!(rig.wt.created.lock().unwrap().is_empty());
}

#[tokio::test]
async fn paid_fallback_off_allows_initial_paid_work_but_blocks_paid_retry() {
    let policy = Arc::new(MeteredPolicy { paid_fallback_off: true, ..MeteredPolicy::default() });
    let rig = policy_rig(&[DeepSeek], policy);
    assert_eq!(rig.d.delegate(brief("x")).await.provider, Some(DeepSeek));

    for paid_fallback_off in [true, false] {
        let policy = Arc::new(MeteredPolicy { paid_fallback_off, ..MeteredPolicy::default() });
        let rig = policy_rig(&[Claude, DeepSeek], policy.clone());
        rig.scripts.lock().unwrap().insert(Claude, QUOTA);
        let result = rig.d.delegate(brief("x")).await;
        assert_eq!(result.ok, !paid_fallback_off, "{:?}", result.error);
        assert_eq!(rig.started.lock().unwrap().len(), if paid_fallback_off { 1 } else { 2 });
        let recorded = policy.recorded.lock().unwrap();
        assert_eq!(recorded.len(), if paid_fallback_off { 1 } else { 2 });
        assert_eq!(recorded[0].0, Claude);
        assert_eq!(recorded[0].2.measurement, Measurement::Unavailable);
    }
}

#[tokio::test]
async fn completed_worker_usage_restricts_the_next_delegation_before_landing() {
    let policy = Arc::new(MeteredPolicy { cap: Some(7), ..MeteredPolicy::default() });
    let rig = policy_rig(&[DeepSeek], policy.clone());
    let finished = rig.d.run(brief("x")).await;
    match finished {
        super::super::outcome::Finished::Pending(pending) => assert!(pending.result.ok),
        super::super::outcome::Finished::Done(result) => panic!("worker did not reach landing: {:?}", result.error),
    }
    let recorded = policy.recorded.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, DeepSeek);
    assert_eq!(recorded[0].1, "deepseek-flash");
    assert_eq!(recorded[0].2.output_tokens, 7);
    assert!(!rig.d.delegate(brief("y")).await.ok);
    assert_eq!(rig.started.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn in_place_read_only_workers_also_record_usage() {
    let policy = Arc::new(MeteredPolicy::default());
    let rig = policy_rig(&[Claude], policy.clone());
    assert!(rig.d.delegate(DelegateRequest { read_only: true, ..brief("x") }).await.ok);
    assert!(rig.wt.created.lock().unwrap().is_empty());
    assert_eq!(policy.recorded.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_worker_reported_usage_is_recorded_and_restricts_the_next_delegation() {
    let policy = Arc::new(MeteredPolicy { cap: Some(7), ..MeteredPolicy::default() });
    let rig = policy_rig(&[DeepSeek], policy.clone());
    rig.scripts.lock().unwrap().insert(DeepSeek, "printf 'FIN:   \\n'");
    assert!(!rig.d.delegate(brief("x")).await.ok);
    let recorded = policy.recorded.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, DeepSeek);
    assert_eq!(recorded[0].1, "deepseek-flash");
    assert_eq!(recorded[0].2, Usage { output_tokens: 7, ..Usage::default() });
    assert!(!rig.d.delegate(brief("y")).await.ok);
    assert_eq!(rig.started.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_worker_without_usage_records_unavailable_instead_of_zero() {
    let policy = Arc::new(MeteredPolicy { cap: Some(7), ..MeteredPolicy::default() });
    let rig = policy_rig(&[DeepSeek], policy.clone());
    rig.scripts.lock().unwrap().insert(DeepSeek, "printf 'FAIL:segmentation fault\\n'");
    assert!(!rig.d.delegate(brief("x")).await.ok);
    let recorded = policy.recorded.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, DeepSeek);
    assert_eq!(recorded[0].1, "deepseek-flash");
    assert_eq!(recorded[0].2, Usage { measurement: Measurement::Unavailable, ..Usage::default() });
    assert!(!rig.d.delegate(brief("y")).await.ok);
    assert_eq!(rig.started.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_model_the_budget_refuses_at_start_is_set_aside_and_another_worker_runs() {
    let policy = Arc::new(MeteredPolicy { refused: vec![(Claude, "claude-sonnet-5-5"), (Claude, "claude-haiku-5-5"), (Claude, "claude-opus-5-5")], ..MeteredPolicy::default() });
    let rig = policy_rig(&[Claude, Codex], policy.clone());
    let result = rig.d.delegate(brief("x")).await;
    assert!(result.ok, "{:?}", result.error);
    assert_eq!(result.provider, Some(Codex));
    assert!(result.retries.iter().any(|r| r.contains("was not started") && r.contains("paid cap would be passed")), "{:?}", result.retries);
    assert_eq!(*policy.held.lock().unwrap(), 0, "every hold ended: settled, none left reserved");
}

#[tokio::test]
async fn when_the_budget_refuses_every_worker_nothing_starts_and_no_worktree_is_made() {
    let policy = Arc::new(MeteredPolicy { refused: vec![(DeepSeek, "deepseek-flash"), (DeepSeek, "deepseek-v4-pro")], ..MeteredPolicy::default() });
    let rig = policy_rig(&[DeepSeek], policy);
    let result = rig.d.delegate(brief("x")).await;
    assert!(!result.ok);
    assert!(rig.started.lock().unwrap().is_empty());
    assert!(rig.wt.created.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_attempt_dropped_half_way_settles_its_hold_as_unknown_consumption() {
    let policy = Arc::new(MeteredPolicy::default());
    let rig = policy_rig(&[DeepSeek], policy.clone());
    rig.scripts.lock().unwrap().insert(DeepSeek, "sleep 30");
    let attempt = rig.d.delegate(brief("x"));
    assert!(tokio::time::timeout(std::time::Duration::from_millis(400), attempt).await.is_err(), "the attempt was still running");
    let recorded = policy.recorded.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0].2.measurement, Measurement::Unavailable, "a cancelled attempt is not free");
    assert_eq!(*policy.held.lock().unwrap(), 0);
}
