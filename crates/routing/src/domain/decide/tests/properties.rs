//! Property tests: a decision never leaves the usable set.

use super::*;

/// Tiny deterministic generator so the property test needs no dependency.
struct Rng(u64);
impl Rng {
fn next(&mut self) -> u64 {
    self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    self.0 >> 33
}
fn unit(&mut self) -> f64 {
    (self.next() % 1001) as f64 / 1000.0
}
fn pick<T: Copy>(&mut self, items: &[T]) -> T {
    items[self.next() as usize % items.len()]
}
}

#[test]
fn a_decision_never_leaves_the_usable_set() {
    let mut rng = Rng(7);
    for mask in 0..32u32 {
        let provs: Vec<_> = PROVIDERS.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, p)| *p).collect();
        for task in TaskType::ALL {
            for round in 0..40 {
                let mut f = Fixture::new(&provs);
                f.allow = Allowances { credits: rng.next().is_multiple_of(2), per_token: rng.next().is_multiple_of(2) };
                f.plan = if rng.next().is_multiple_of(2) { Some(rng.unit()) } else { None };
                f.context_tokens = rng.pick(&[0, 300_000, 700_000]);
                f.pin = match rng.next() % 6 {
                    0 => Pin::Provider(rng.pick(&PROVIDERS)),
                    1 => {
                        let m = rng.pick(&models().iter().collect::<Vec<_>>());
                        Pin::Model(m.provider, m.id.to_string())
                    }
                    _ => Pin::Auto,
                };
                let s = Signals {
                    task,
                    task_confidence: rng.unit(),
                    very_hard_p: rng.unit(),
                    security_p: rng.unit(),
                    edit_p: rng.unit(),
                    continues_p: rng.unit(),
                    correction_p: if rng.next().is_multiple_of(2) { Some(rng.unit()) } else { None },
                    orchestration: Orchestration::default(),
                    security_keywords: rng.pick(&[Security::None, Security::Weak, Security::Strong]),
                };
                let signals = (round % 5 != 0).then_some(&s);
                // Start from a random session, including a model that has since become unusable.
                let mut st = RoutingState::default();
                if !rng.next().is_multiple_of(3) {
                    record_turn(&mut st, rng.pick(&models().iter().collect::<Vec<_>>()));
                }
                for _ in 0..3 {
                    let words = rng.pick(&[3, 9, 30]);
                    if let Ok(d) = f.run(&mut st, signals, words) {
                        assert!(provs.contains(&d.spec.provider), "mask {mask} {task} {:?}", d.spec.id);
                        assert!(f.usable.contains(d.spec));
                        assert!(allowed_for(d.task, d.spec), "{} for {}", d.spec.id, d.task);
                        assert!(d.spec.context_tokens >= f.context_tokens);
                        assert!(match d.spec.billing {
                            Billing::Subscription => true,
                            Billing::Credits => f.allow.credits,
                            Billing::PerToken => f.allow.per_token,
                        });
                        assert!(!d.reason.is_empty());
                        record_turn(&mut st, d.spec);
                    }
                }
            }
        }
    }
}

#[test]
fn auto_with_no_failure_always_routes_when_something_in_the_chain_is_usable() {
    // The non-error side of the property: a usable chain entry means an answer.
    let f = Fixture::new(&[Claude]);
    for task in TaskType::ALL {
        let chain_has_claude = default_chain(task, false).iter().any(|s| s.provider == Claude && s.billing != Billing::Credits);
        let got = f.run(&mut RoutingState::default(), Some(&sig(task)), 20);
        assert_eq!(got.is_ok(), chain_has_claude, "{task}");
    }
}
