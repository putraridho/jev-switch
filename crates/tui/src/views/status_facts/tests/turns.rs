use super::*;
use jev_backend::Usage;
use jev_changes::domain::change_set::ChangeSummary;
use jev_changes::domain::file_change::{ChangeStatus, FileChange as GitChange};
use jev_turn::application::turn_report::TurnReport;
use jev_session::domain::turn_status::TurnStatus;
use super::super::turns::{exit_code, file_changes, summary_facts};

#[test]
fn the_summary_comes_from_the_report() {
    let mut rt = runtime();
    rt.note_plan(Claude, jev_backend::PlanUsage { window: "five_hour".into(), utilization: 0.32, resets_at: None });
    let report = TurnReport {
        status: TurnStatus::Answered,
        text: "x".into(),
        message: String::new(),
        provider: Some(Claude),
        model: Some("claude-sonnet-5-5".into()),
        usage: Some(Usage { input_tokens: 45_000, cached_input_tokens: 40_000, output_tokens: 189, context_tokens: None, ..Usage::default() }),
        attempts: 1,
        tool_calls: 0,
        changes: Some(ChangeSummary {
            files: vec![GitChange { path: "a.rs".into(), status: ChangeStatus::Modified, added: 12, removed: 4, old_path: None }],
            before: "b".into(),
            after: "a".into(),
            added_stamps: Default::default(),
            ..ChangeSummary::default()
        }),
        acceptance: None,
    };
    let f = summary_facts(&report, 14_000, &rt);
    assert!(f.ok && !f.cancelled);
    assert_eq!((f.provider.as_str(), f.model.as_str()), ("claude", "claude-sonnet-5-5"));
    assert_eq!((f.in_tokens, f.cached_tokens, f.out_tokens), (45_000, 40_000, 189));
    assert_eq!(f.plan, Some(("claude".to_string(), 0.32)));
    assert_eq!(f.files_changed, Some((1, 12, 4)));

    let stopped = TurnReport { status: TurnStatus::Cancelled, usage: None, changes: None, provider: None, model: None, ..report };
    let f = summary_facts(&stopped, 3000, &rt);
    assert!(f.cancelled && !f.ok && f.plan.is_none() && f.files_changed.is_none());
}

#[test]
fn changed_files_map_to_status_letters() {
    let s = ChangeSummary {
        files: [ChangeStatus::Added, ChangeStatus::Modified, ChangeStatus::Deleted, ChangeStatus::Renamed]
            .into_iter()
            .map(|status| GitChange { path: "p".into(), status, added: 1, removed: 2, old_path: None })
            .collect(),
        before: String::new(),
        after: String::new(),
        added_stamps: Default::default(),
        ..ChangeSummary::default()
    };
    assert_eq!(file_changes(&s).iter().map(|f| f.status).collect::<String>(), "AMDR");
}

#[test]
fn exit_codes_follow_the_shell_conventions() {
    assert_eq!(exit_code(TurnStatus::Answered), 0);
    assert_eq!(exit_code(TurnStatus::Failed), 1);
    assert_eq!(exit_code(TurnStatus::NoRoute), 1);
    assert_eq!(exit_code(TurnStatus::Cancelled), 130);
}
