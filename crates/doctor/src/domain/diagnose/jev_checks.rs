//! The Jev key and whether Jev answers.

use super::{check, steps, Check, Inputs, Severity};

pub(super) fn jev_key_check(present: bool, env_path: &str) -> Check {
    if present {
        check("jev-key", "Jev key found", Severity::Ok, "Routing can ask Jev which model fits each task.", Vec::new())
    } else {
        check(
            "jev-key",
            "Jev key is missing",
            Severity::Problem,
            "Routing needs a Jev key to judge each task.",
            vec![format!("Add JEV_API_KEY=... to {env_path}")],
        )
    }
}

/// Left out when the key is missing (nothing to test) or the ping was not run.
pub(super) fn jev_reachable_check(inputs: &Inputs) -> Option<Check> {
    if !inputs.jev_key_present {
        return None;
    }
    Some(match inputs.jev_reachable? {
        true => check("jev-reachable", "Jev is reachable", Severity::Ok, "A test request was answered.", Vec::new()),
        false => check(
            "jev-reachable",
            "Jev did not answer",
            Severity::Warn,
            "A tiny test request failed or took longer than 3 seconds.",
            steps(&["Check your network connection", "Check that the key in JEV_API_KEY is valid and not expired"]),
        ),
    })
}
