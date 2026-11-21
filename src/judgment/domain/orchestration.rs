use serde_json::{json, Value};

use super::thresholds::SEQUENTIAL_VETO_P;

/// Jev's judgment of whether a request should be orchestrated: the two raw answers and the
/// veto they imply. The veto is a decision of its own, not a lowered score: a cutoff set
/// at or under any cap would let a sequence through, so the gate ([`crate::orchestration::domain::orchestration_gate::orch_gate`])
/// refuses a vetoed request whatever the cutoff is.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Orchestration {
    /// Jev's `multi_part` answer: what the cutoff is compared with.
    pub multi_part_p: f64,
    /// Jev's `sequential` answer; `None` when it was not asked (orchestration is off) or
    /// could not be read, which leaves no veto information: only `multi_part` decides.
    pub sequential_p: Option<f64>,
    /// The request is a sequence of dependent steps (`sequential_p` at [`SEQUENTIAL_VETO_P`]
    /// or more): parallel workers gain nothing, so it is never orchestrated.
    pub vetoed: bool,
}

impl Orchestration {
    pub fn new(multi_part_p: f64, sequential_p: f64) -> Self {
        Self::judged(multi_part_p, Some(sequential_p))
    }

    /// The judgment from the answers there are: without a `sequential` answer nothing is vetoed.
    pub fn judged(multi_part_p: f64, sequential_p: Option<f64>) -> Self {
        Self { multi_part_p, sequential_p, vetoed: sequential_p.is_some_and(|p| p >= SEQUENTIAL_VETO_P) }
    }

    /// The fields of an orchestrate check's log row and response; all null when Jev gave
    /// no answer (`None`). The raw probabilities and the veto are all logged, so a report can
    /// say what a cutoff would have triggered and what the veto held back.
    pub fn fields(judgment: Option<&Self>) -> Value {
        json!({
            "multi_part_p": judgment.map(|j| j.multi_part_p),
            "sequential_p": judgment.map(|j| j.sequential_p),
            "vetoed": judgment.map(|j| j.vetoed),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parallel_request_is_not_vetoed_and_no_answer_logs_nulls() {
        let j = Orchestration::new(0.9, 0.2);
        assert_eq!((j.multi_part_p, j.vetoed), (0.9, false));
        assert_eq!(Orchestration::fields(Some(&j))["vetoed"], false);
        let none = Orchestration::fields(None);
        for key in ["multi_part_p", "sequential_p", "vetoed"] {
            assert!(none[key].is_null(), "{key}");
        }
    }

    #[test]
    fn a_sequence_is_vetoed_and_parallel_work_is_not() {
        assert!(!Orchestration::new(0.89, 0.10).vetoed, "independent pieces");
        assert!(!Orchestration::new(0.89, 0.69).vetoed, "below the veto");
        assert!(Orchestration::new(0.89, 0.90).vetoed, "do 1 -> 2 -> 3 is not orchestrated");
        assert_eq!(Orchestration::new(0.89, 0.90).multi_part_p, 0.89, "the raw answer is never rewritten");
    }
}
