//! Routing-only dry run: classify many prompts with Jev (no Claude calls) and
//! report where each would route at CHEAP_THRESHOLD. Cheap way to see the
//! confidence distribution and find borderline cases worth labeling for `tune`.
//!
//! Input: one prompt per line, optionally `category<TAB>prompt`.
//! Output: TSV on stdout (category, kind, confidence, model, prompt); summary on stderr.
//!
//!   TYPESAFE_API_KEY=... CHEAP_THRESHOLD=0.30 cargo run --bin route prompts.tsv

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use jev_router::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use jev_router::router::{Decision, Router};

const TIERS: [&str; 3] = [MODEL_CHEAP, MODEL_MID, MODEL_STRONG];
use jev_router::typesafe::TypeSafe;

const CONCURRENCY: usize = 8;

fn env_or(key: &str, default: f64) -> f64 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("usage: route <prompts file>")?;
    let threshold: f64 = std::env::var("CHEAP_THRESHOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.50); // calibrated: `tune` + 1000-prompt run

    // Sonnet matched Opus on every sampled prompt below this; see README.
    let strong: f64 = std::env::var("STRONG_THRESHOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.85);

    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path))?;
    let prompts: Vec<(String, String)> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once('\t') {
            Some((cat, p)) => (cat.trim().to_string(), p.trim().to_string()),
            None => ("-".to_string(), l.to_string()),
        })
        .collect();

    // ROUTE_MODE=code dry-runs code mode's first-turn policy (kind column = reason),
    // with code mode's own cutoffs.
    let code_mode = std::env::var("ROUTE_MODE").as_deref() == Ok("code");
    let (threshold, strong) = if code_mode {
        (env_or("CODE_CHEAP_THRESHOLD", 0.60), env_or("CODE_STRONG_THRESHOLD", 0.80))
    } else {
        (threshold, strong)
    };
    let router = Arc::new(Router::new(TypeSafe::from_env()?, threshold).with_strong_threshold(strong));
    let sem = Arc::new(Semaphore::new(CONCURRENCY));
    let mut tasks = JoinSet::new();
    for (i, (cat, prompt)) in prompts.into_iter().enumerate() {
        let (router, sem) = (router.clone(), sem.clone());
        let code_mode = code_mode;
        tasks.spawn(async move {
            let _permit = sem.acquire_owned().await;
            let d = if code_mode {
                let w = router.code_want(&prompt, "", 0, 1).await;
                Ok(Decision { model: TIERS[w.tier], kind: w.reason.clone(), confidence: 0.0, reason: "code" })
            } else {
                router.route(&prompt, "").await
            };
            (i, cat, prompt, d)
        });
    }

    let mut rows = Vec::new();
    while let Some(res) = tasks.join_next().await {
        rows.push(res?);
    }
    rows.sort_by_key(|r| r.0);

    let tier = |m: &str| TIERS.iter().position(|x| *x == m).unwrap_or(2);
    // category -> count per tier
    let mut by_cat: BTreeMap<String, [usize; 3]> = BTreeMap::new();
    let mut failed = 0;
    println!("category\tkind\tconfidence\tmodel\treason\tprompt");
    for (_, cat, prompt, d) in &rows {
        match d {
            Ok(d) => {
                println!("{}\t{}\t{:.2}\t{}\t{}\t{}", cat, d.kind, d.confidence, d.model, d.reason, prompt);
                by_cat.entry(cat.clone()).or_default()[tier(d.model)] += 1;
            }
            Err(e) => {
                failed += 1;
                eprintln!("  error on {:?}: {:#}", prompt, e);
            }
        }
    }

    let pct = |n: usize, t: usize| 100.0 * n as f64 / t.max(1) as f64;
    let mut all = [0usize; 3];
    eprintln!("\ncheap≥{} strong≥{} — {} failed\n  {:<15} {:>7} {:>7} {:>7}", threshold, strong, failed, "category", "haiku", "sonnet", "opus");
    for (cat, n) in &by_cat {
        let t: usize = n.iter().sum();
        (0..3).for_each(|i| all[i] += n[i]);
        eprintln!("  {:<15} {:>6.0}% {:>6.0}% {:>6.0}%", cat, pct(n[0], t), pct(n[1], t), pct(n[2], t));
    }
    let t: usize = all.iter().sum();
    eprintln!("  {:<15} {:>6.0}% {:>6.0}% {:>6.0}%   ({} prompts)", "ALL", pct(all[0], t), pct(all[1], t), pct(all[2], t), t);
    Ok(())
}
