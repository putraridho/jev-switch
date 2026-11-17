//! Starting cutoff for orchestrate mode's auto-trigger: score the prompts the gateway
//! has logged with Jev's `multi_part` and `sequential` judgment and show how many would
//! trigger at each cutoff, with example prompts per score band, so the cutoff is read off
//! real prompts. A prompt the sequence veto holds back keeps its raw `multi_part` in the
//! output, marked `vetoed`, and is never counted as triggering at any cutoff.
//!
//!   cargo run --bin orch_tune [gateway.jsonl] > scores.tsv
//!
//! Input: the gateway's decision log (default `~/.local/state/jev-router/gateway.jsonl`).
//! It keeps only the first 200 characters of each prompt and no conversation context,
//! so long prompts are scored on their start. Output, highest trigger score first:
//! `multi_part<TAB>sequential<TAB>veto<TAB>prompt` on stdout (see [`columns`]; `veto` is
//! `vetoed` or `-`); the distribution and examples on stderr.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use jev_router::log;
use jev_router::router::{Orchestration, Router};
use jev_router::typesafe::TypeSafe;

const CONCURRENCY: usize = 8;
const EXAMPLES_PER_BAND: usize = 6;

/// Older log lines include messages Claude Code puts in the user slot (subagent
/// hand-backs, permission notices); they were never typed and would skew the curve.
fn is_harness(prompt: &str) -> bool {
    let p = prompt.trim_start();
    ["Another Claude session sent a message", "Permission granted for", "<agent-message", "<task-notification"]
        .iter()
        .any(|x| p.starts_with(x))
}

/// `multi_part  sequential  veto`, tab separated: Jev's two answers and whether the sequence
/// veto holds the prompt back (`vetoed`) however high `multi_part` is. A `sequential` answer
/// Jev did not give is `-`.
fn columns(j: &Orchestration) -> String {
    let sequential = j.sequential_p.map_or_else(|| "-".to_string(), |p| format!("{p:.3}"));
    format!("{:.3}\t{sequential}\t{}", j.multi_part_p, if j.vetoed { "vetoed" } else { "-" })
}

/// What the trigger would see at a cutoff: `multi_part`, or nothing for a vetoed prompt.
fn trigger_score(j: &Orchestration) -> f64 {
    if j.vetoed {
        f64::NEG_INFINITY
    } else {
        j.multi_part_p
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => log::path_from_env("JEV_GATEWAY_LOG", "gateway.jsonl").context("no state dir")?,
    };
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {path}"))?;
    let mut seen = HashSet::new();
    let prompts: Vec<String> = raw
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| v.get("prompt").and_then(Value::as_str).map(|s| s.trim().to_string()))
        .filter(|p| !p.is_empty() && !is_harness(p) && seen.insert(p.clone()))
        .collect();
    eprintln!("scoring {} distinct prompts from {path}", prompts.len());

    let router = Arc::new(Router::new(TypeSafe::from_env()?, 1.0));
    let sem = Arc::new(Semaphore::new(CONCURRENCY));
    let mut tasks = JoinSet::new();
    for prompt in prompts {
        let (router, sem) = (router.clone(), sem.clone());
        tasks.spawn(async move {
            let _permit = sem.acquire_owned().await;
            let judgment = router.orchestration_p(&prompt, "").await;
            (prompt, judgment)
        });
    }
    let mut scored = Vec::new();
    let mut failed = 0;
    while let Some(res) = tasks.join_next().await {
        match res? {
            (prompt, Ok(j)) => scored.push((j, prompt)),
            (prompt, Err(e)) => {
                failed += 1;
                eprintln!("  error on {:?}: {e:#}", prompt.chars().take(60).collect::<String>());
            }
        }
    }
    scored.sort_by(|a, b| trigger_score(&b.0).total_cmp(&trigger_score(&a.0)));
    for (j, prompt) in &scored {
        println!("{}\t{}", columns(j), prompt.replace(['\n', '\t'], " "));
    }

    let n = scored.len().max(1);
    eprintln!("\n{} scored, {failed} failed\n\ncutoff  would trigger", scored.len());
    for cut in [0.5, 0.6, 0.7, 0.75, 0.8, 0.85, 0.9, 0.95] {
        let k = scored.iter().filter(|(j, _)| trigger_score(j) >= cut).count();
        eprintln!("  {cut:.2}  {k:>5}  ({:.1}%)", 100.0 * k as f64 / n as f64);
    }
    for (lo, hi) in [(0.9, 1.01), (0.8, 0.9), (0.7, 0.8), (0.6, 0.7), (0.5, 0.6)] {
        let band: Vec<_> = scored.iter().filter(|(j, _)| trigger_score(j) >= lo && trigger_score(j) < hi).collect();
        eprintln!("\n[{lo:.1}, {:.1}) — {} prompts", hi.min(1.0), band.len());
        // Spread the examples across the band rather than taking its top.
        let step = (band.len() / EXAMPLES_PER_BAND).max(1);
        for (j, prompt) in band.iter().step_by(step).take(EXAMPLES_PER_BAND) {
            eprintln!("  {}  {}", columns(j), prompt.replace('\n', " ").chars().take(150).collect::<String>());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vetoed_prompt_shows_its_raw_multi_part_and_the_veto_and_never_triggers() {
        assert_eq!(columns(&Orchestration::new(0.95, 0.9)), "0.950\t0.900\tvetoed");
        assert_eq!(columns(&Orchestration::new(0.8, 0.1)), "0.800\t0.100\t-");
        assert!(trigger_score(&Orchestration::new(0.95, 0.9)) < 0.0);
        assert_eq!(trigger_score(&Orchestration::new(0.8, 0.1)), 0.8);
        assert_eq!(columns(&Orchestration::judged(0.8, None)), "0.800\t-\t-");
    }
}
