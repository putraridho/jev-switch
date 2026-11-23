use super::*;
use jev_catalog::domain::model_table::{default_chain, find, Tier};

fn spec(id: &str) -> &'static ModelSpec {
    jev_catalog::domain::model_table::models().iter().find(|m| m.id == id).unwrap()
}

fn rec(accepted: u32, verified: u32, tokens: Option<u64>) -> TrackRecord {
    TrackRecord { verified, accepted, tokens_per_accepted: tokens }
}

fn records(list: &[(&str, TaskType, TrackRecord)]) -> TrackRecords {
    let mut r = TrackRecords::default();
    for (id, task, record) in list {
        r.insert(spec(id).provider, id, *task, *record);
    }
    r
}

fn ids(chain: &[&'static ModelSpec]) -> Vec<&'static str> {
    chain.iter().map(|s| s.id).collect()
}

#[test]
fn with_no_data_every_static_chain_is_unchanged() {
    let none = TrackRecords::default();
    for task in TaskType::ALL {
        for very_hard in [false, true] {
            let chain = default_chain(task, very_hard);
            assert_eq!(ids(&order_by_evidence(&chain, task, &none)), ids(&chain), "{task}");
        }
    }
}

#[test]
fn the_interval_is_wide_for_few_samples_and_narrow_for_many() {
    let few = Rate::of(2, 2).unwrap();
    let many = Rate::of(200, 200).unwrap();
    assert!(few.low < 0.4 && many.low > 0.97, "{few:?} {many:?}");
    assert_eq!(Rate::of(0, 0), None);
    let half = Rate::of(5, 10).unwrap();
    assert!(half.low < 0.5 && half.high > 0.5 && (half.point - 0.5).abs() < 1e-9);
}

/// Two adjacent models of one tier in the Feature chain.
fn pair() -> (Vec<&'static ModelSpec>, &'static str, &'static str) {
    let chain = default_chain(TaskType::Feature, false);
    let i = (0..chain.len() - 1).find(|i| chain[*i].tier == chain[*i + 1].tier).expect("a same-tier pair");
    (chain.clone(), chain[i].id, chain[i + 1].id)
}

#[test]
fn a_clearly_better_equal_tier_neighbour_moves_first() {
    let (chain, a, b) = pair();
    let r = records(&[(a, TaskType::Feature, rec(2, 10, Some(1000))), (b, TaskType::Feature, rec(10, 10, Some(1000)))]);
    let ordered = ids(&order_by_evidence(&chain, TaskType::Feature, &r));
    let (pa, pb) = (ordered.iter().position(|x| *x == a).unwrap(), ordered.iter().position(|x| *x == b).unwrap());
    assert!(pb < pa, "{ordered:?}");
}

#[test]
fn too_few_verified_turns_change_nothing() {
    let (chain, a, b) = pair();
    let r = records(&[(a, TaskType::Feature, rec(0, 4, None)), (b, TaskType::Feature, rec(4, 4, None))]);
    assert_eq!(ids(&order_by_evidence(&chain, TaskType::Feature, &r)), ids(&chain));
    // One model with a record and one without: no comparison.
    let one = records(&[(b, TaskType::Feature, rec(50, 50, None))]);
    assert_eq!(ids(&order_by_evidence(&chain, TaskType::Feature, &one)), ids(&chain));
}

#[test]
fn the_same_band_goes_to_the_cheaper_per_accepted_task_and_then_the_chain() {
    let (chain, a, b) = pair();
    let cheap = records(&[(a, TaskType::Feature, rec(10, 10, Some(9000))), (b, TaskType::Feature, rec(10, 10, Some(2000)))]);
    let ordered = ids(&order_by_evidence(&chain, TaskType::Feature, &cheap));
    assert!(ordered.iter().position(|x| *x == b) < ordered.iter().position(|x| *x == a));
    let same = records(&[(a, TaskType::Feature, rec(10, 10, Some(2000))), (b, TaskType::Feature, rec(10, 10, Some(2000)))]);
    assert_eq!(ids(&order_by_evidence(&chain, TaskType::Feature, &same)), ids(&chain));
}

#[test]
fn records_of_another_task_type_do_not_count() {
    let (chain, a, b) = pair();
    let r = records(&[(a, TaskType::Docs, rec(0, 10, None)), (b, TaskType::Docs, rec(10, 10, None))]);
    assert_eq!(ids(&order_by_evidence(&chain, TaskType::Feature, &r)), ids(&chain));
}

#[test]
fn a_different_tier_is_never_passed() {
    // Whatever the records say, the tier sequence of the chain stays the same.
    let chain = default_chain(TaskType::Feature, false);
    let mut r = TrackRecords::default();
    for (i, s) in chain.iter().enumerate() {
        r.insert(s.provider, s.id, TaskType::Feature, rec(if i == 0 { 5 } else { 10 }, 10, None));
    }
    let tiers = |c: &[&'static ModelSpec]| c.iter().map(|s| s.tier).collect::<Vec<Tier>>();
    assert_eq!(tiers(&order_by_evidence(&chain, TaskType::Feature, &r)), tiers(&chain));
}

#[test]
fn security_work_keeps_its_static_order() {
    let chain = default_chain(TaskType::Security, true);
    let mut r = TrackRecords::default();
    for (i, s) in chain.iter().enumerate() {
        r.insert(s.provider, s.id, TaskType::Security, rec(if i == 0 { 0 } else { 10 }, 10, None));
    }
    assert_eq!(ids(&order_by_evidence(&chain, TaskType::Security, &r)), ids(&chain));
    assert!(find(spec("claude-fable-5-2").provider, "claude-fable-5-2").is_some());
}
