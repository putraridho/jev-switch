//! Readiness checks: how far up the ladder each provider is, and how to fix the rung that failed.

use jev_providers::domain::readiness_view::ReadinessView;

use super::{check, Check, Severity};

/// A warning per provider with a failed rung (what failed, when it is tried again, the fix),
/// and one note with every ladder.
pub(super) fn readiness_checks(views: &[ReadinessView]) -> Vec<Check> {
    let mut checks: Vec<Check> = views
        .iter()
        .filter_map(|v| {
            let (rung, _) = v.failed.as_ref()?;
            Some(check("readiness-failed", format!("{} is held: {} failed", v.provider, rung.label()), Severity::Warn, v.line.clone(), v.fix.clone()))
        })
        .collect();
    if !views.is_empty() {
        let detail = views.iter().map(|v| v.line.clone()).collect::<Vec<_>>().join("\n");
        checks.push(check("readiness", "Provider readiness", Severity::Info, detail, Vec::new()));
    }
    checks
}

/// What the capability handshake found. It sends no prompt and spends no quota.
pub(super) fn handshake_check(lines: &[String]) -> Option<Check> {
    (!lines.is_empty()).then(|| {
        check("handshake", "Capability handshake (no prompt sent, no quota spent)", Severity::Info, lines.join("\n"), Vec::new())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_providers::domain::availability::{Availability, State};
    use jev_providers::domain::breaker::Admission;
    use jev_providers::domain::readiness::{FailureClass, Ladder, Readiness, ReadinessKey, RetryPolicy, Rung, Scope};
    use jev_providers::domain::readiness_view::describe;
    use jev_shared::ProviderId;

    const NOW: u64 = 1_760_000_000_000;

    fn view(rung: Rung) -> ReadinessView {
        let a = Availability { provider: ProviderId::Gemini, state: State::Ready, plan: None, models: None, detail: "agy login".into(), verified: false };
        let (mut r, k) = (Readiness::default(), ReadinessKey::default());
        r.fail(ProviderId::Gemini, "p", &k, FailureClass { rung, policy: RetryPolicy::UntilChanged, scope: Scope::Provider }, "refused", NOW);
        describe(ProviderId::Gemini, &Ladder::build(&a, NOW, r.record(ProviderId::Gemini, "p", &k), NOW + 1), Admission::Closed, NOW + 1)
    }

    #[test]
    fn each_failed_rung_says_how_to_fix_it() {
        for (rung, fix) in [(Rung::Authenticated, "Log in"), (Rung::ProtocolCompatible, "Update the CLI"), (Rung::ValidatedForProject, "Trust this folder")] {
            let checks = readiness_checks(&[view(rung)]);
            let failed = checks.iter().find(|c| c.id == "readiness-failed").unwrap();
            assert_eq!(failed.severity, Severity::Warn);
            assert!(failed.fix.join(" ").contains(fix), "{:?}", failed.fix);
            assert!(failed.detail.contains("held until"), "{}", failed.detail);
        }
        assert!(handshake_check(&[]).is_none());
        assert!(handshake_check(&["gemini: ok".into()]).unwrap().title.contains("no quota spent"));
    }
}
