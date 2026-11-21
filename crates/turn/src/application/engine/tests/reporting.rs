//! The structured callbacks a UI gets, and small pure helpers.

use super::*;

/// A UI that overrides every structured callback and records what each one got.
#[derive(Default)]
struct Structured {
    log: Vec<String>,
}

impl Ui for Structured {
    fn route(&mut self, _: &str) {
        self.log.push("route-string".into());
    }
    fn event(&mut self, _: &Event) {}
    fn note(&mut self, line: &str) {
        self.log.push(format!("note:{line}"));
    }
    fn route_facts(&mut self, f: &RouteFacts) {
        self.log.push(format!("route:{}:{}:{}", f.provider, f.model, f.task));
    }
    fn failover(&mut self, f: &Failover) {
        self.log.push(format!("failover:{}:{:?}:retry={}", f.provider, f.kind, f.will_retry));
    }
    fn no_route(&mut self, info: &NoRouteInfo, _: &str) {
        self.log.push(match info {
            NoRouteInfo::Pin { .. } => "no-route:pin".into(),
            NoRouteInfo::Nothing { task } => format!("no-route:{task}"),
            NoRouteInfo::Refused { .. } => "no-route:refused".into(),
        });
    }
    fn cancelled(&mut self, _: &str) {
        self.log.push("cancelled".into());
    }
}

#[tokio::test]
async fn structured_callbacks_carry_the_facts_and_replace_the_string_ones() {
    let ready = [Claude, Codex];
    let mut r = rig(&ready);
    let a = first_choice(TaskType::Feature, &ready);
    r.script(a, QUOTA);
    let mut ui = Structured::default();
    let report = r.engine.run_judged("add a retry", Judged::Providers(Some(want(TaskType::Feature))), &mut ui).await;
    assert_eq!(report.status, TurnStatus::Answered);
    let b = report.provider.unwrap();
    assert_eq!(ui.log.len(), 3, "{:?}", ui.log);
    assert!(ui.log[0].starts_with(&format!("route:{a}:")) && ui.log[0].ends_with(":feature"), "{:?}", ui.log);
    assert_eq!(ui.log[1], format!("failover:{a}:Quota {{ resets_at: Some(\"1794853451\") }}:retry=true"));
    assert!(ui.log[2].starts_with(&format!("route:{b}:")), "{:?}", ui.log);
    assert!(!ui.log.iter().any(|l| l == "route-string"), "the string callback is not used when the structured one is overridden");
}

#[tokio::test]
async fn a_final_failure_and_a_missing_route_are_reported_structurally() {
    let mut r = rig(&[Claude]);
    r.script(Claude, OTHER);
    let mut ui = Structured::default();
    let report = r.engine.run_judged("add a retry", Judged::Providers(Some(want(TaskType::Feature))), &mut ui).await;
    assert_eq!(report.status, TurnStatus::Failed);
    assert_eq!(ui.log.last().unwrap(), "failover:claude:Other:retry=false");

    let mut none = rig(&[]);
    let mut ui = Structured::default();
    let report = none.engine.run_judged("add a retry", Judged::Providers(Some(want(TaskType::Feature))), &mut ui).await;
    assert_eq!(report.status, TurnStatus::NoRoute);
    assert_eq!(ui.log, vec!["no-route:feature".to_string()]);
}

#[test]
fn idle_timeout_reads_positive_seconds_and_defaults_otherwise() {
    assert_eq!(idle_timeout(None), Duration::from_secs(900));
    assert_eq!(idle_timeout(Some(" 30 ")), Duration::from_secs(30));
    for bad in ["", "abc", "0", "-5", "1.5"] {
        assert_eq!(idle_timeout(Some(bad)), Duration::from_secs(900), "{bad:?}");
    }
}

#[test]
fn git_status_short_is_empty_outside_a_repo() {
    assert_eq!(git_status_short(Path::new("/definitely/not/a/dir")), "");
}
