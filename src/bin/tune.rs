//! Calibrate the cheap-route threshold from data instead of guessing.
//!
//! Reads a labeled set (default `eval/cases.json`), asks Jev to classify each
//! request once, then sweeps the confidence cutoff and prints, per cutoff:
//!   - % routed to the cheap model (your savings), and
//!   - miss rate = of those cheap routes, the fraction the cheap model would NOT
//!     have handled well (the costly error).
//! It then suggests the LOWEST cutoff whose miss rate is within MISS_BUDGET
//! (default 0.05) — i.e. maximum savings inside your error budget.
//!
//!   ANTHROPIC_API_KEY unused here. TYPESAFE_API_KEY=... cargo run --bin tune [cases.json]
//!
//! This calls Jev once per case (tokens cost money); the set is small on purpose.

use std::fs;

use anyhow::{Context, Result};
use serde::Deserialize;

use jev_router::router::Router;
use jev_router::typesafe::TypeSafe;

/// One labeled example. `cheap_was_fine` is the ground truth that matters:
/// did the CHEAP model actually produce a good-enough answer for this request?
#[derive(Debug, Deserialize)]
struct Case {
    request: String,
    cheap_was_fine: bool,
    #[serde(default)]
    note: String,
}

struct Scored {
    choice: String,
    confidence: f64,
    cheap_was_fine: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "eval/cases.json".into());
    let miss_budget: f64 = std::env::var("MISS_BUDGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.05);

    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path))?;
    let cases: Vec<Case> = serde_json::from_str(&raw).context("parse cases json")?;
    anyhow::ensure!(!cases.is_empty(), "no cases in {}", path);

    let router = Router::new(TypeSafe::from_env()?, 0.0);

    eprintln!("classifying {} cases via Jev…", cases.len());
    let mut scored = Vec::with_capacity(cases.len());
    for (i, c) in cases.iter().enumerate() {
        let ans = router.classify(&c.request, "").await?;
        eprintln!(
            "  {:>3}. {:<10} conf {:.2}  cheap_ok={}  {}",
            i + 1,
            ans.choice,
            ans.confidence,
            c.cheap_was_fine,
            if c.note.is_empty() { &c.request } else { &c.note }
        );
        scored.push(Scored { choice: ans.choice, confidence: ans.confidence, cheap_was_fine: c.cheap_was_fine });
    }

    let total = scored.len() as f64;
    println!("\n  cutoff | routed_cheap | savings% | miss_rate | missed");
    println!("  -------+--------------+----------+-----------+-------");
    let mut suggested: Option<f64> = None;
    let mut t = 0.0_f64;
    while t <= 1.0001 {
        // route to cheap only when judged simple AND confidence >= cutoff
        let cheap: Vec<&Scored> = scored
            .iter()
            .filter(|s| s.choice == "simple" && s.confidence >= t)
            .collect();
        let n_cheap = cheap.len();
        let missed = cheap.iter().filter(|s| !s.cheap_was_fine).count();
        let miss_rate = if n_cheap == 0 { 0.0 } else { missed as f64 / n_cheap as f64 };
        let savings = n_cheap as f64 / total * 100.0;
        println!(
            "   {:.2}  |     {:>3}      |  {:>5.1}   |   {:.2}    |  {:>3}",
            t, n_cheap, savings, miss_rate, missed
        );
        // lowest cutoff within the miss budget that still routes something cheap
        if suggested.is_none() && n_cheap > 0 && miss_rate <= miss_budget {
            suggested = Some(t);
        }
        t += 0.05;
    }

    match suggested {
        Some(t) => println!(
            "\n  Suggested CHEAP_THRESHOLD = {:.2}  (lowest cutoff with miss_rate \u{2264} {:.2})",
            t, miss_budget
        ),
        None => println!(
            "\n  No cutoff keeps miss_rate \u{2264} {:.2} while routing anything cheap.\n  → send everything to the strong model, add more/cleaner cases, or sharpen the criteria.",
            miss_budget
        ),
    }
    println!("  Set it:  CHEAP_THRESHOLD=<value> cargo run --bin chat");
    Ok(())
}
