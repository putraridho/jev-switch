use super::*;
use crate::views::agent_workers::timeline::*;

#[test]
fn timeline_shows_overlap_waits_and_the_critical_path() {
    let spans = [
        Span { id: "w1".into(), start_ms: 0, end_ms: 10_000, waited: None },
        Span { id: "w2".into(), start_ms: 2_000, end_ms: 20_000, waited: Some((10_000, 14_000)) },
    ];
    let t = timeline(&spans, 60, OFF);
    let rows: Vec<&str> = t.lines().collect();
    assert!(rows[0].contains("20s"));
    assert!(rows[1].contains('█') && !rows[1].contains("critical"));
    assert!(rows[2].contains('░') && rows[2].contains("critical path"));
    assert!(timeline(&[], 60, OFF).contains("No timeline"));
}
