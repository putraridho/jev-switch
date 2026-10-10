//! Routing-only dry run: classify many prompts with Jev (no Claude calls) and
//! report where each would route at CHEAP_THRESHOLD. Cheap way to see the
//! confidence distribution and find borderline cases worth labeling for `tune`.
//!
//! Input: one prompt per line, optionally `category<TAB>prompt`; or a `.jsonl` file of
//! `{id, category, prompt, context}` objects (multi-line prompts; `context` is passed
//! to Jev as the recent conversation).
//! Output: TSV on stdout (id, category, kind, confidence, model, reason, prompt with
//! newlines escaped); summary on stderr.
//!
//!   TYPESAFE_API_KEY=... CHEAP_THRESHOLD=0.30 cargo run --bin route prompts.tsv

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use jev_router::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use jev_router::router::{security_level, Decision, Router, CODE_STRONG_THRESHOLD};

const TIERS: [&str; 3] = [MODEL_CHEAP, MODEL_MID, MODEL_STRONG];
use jev_router::typesafe::TypeSafe;

const CONCURRENCY: usize = 8;

#[derive(serde::Deserialize)]
struct Item {
    id: String,
    #[serde(default = "dash")]
    category: String,
    prompt: String,
    #[serde(default)]
    context: String,
}

fn dash() -> String {
    "-".into()
}

/// All code-mode inputs for one prompt as JSON: keyword level and the three Jev
/// judgments (null when Jev failed).
async fn code_signals(router: &Router, prompt: &str, context: &str) -> String {
    let (c, e, s) = tokio::join!(
        router.classify(prompt, context),
        router.needs_changes(prompt, context),
        router.security_review(prompt, context)
    );
    serde_json::json!({
        "kw": format!("{:?}", security_level(prompt)),
        "choice": c.as_ref().ok().map(|a| a.choice.clone()),
        "conf": c.as_ref().ok().map(|a| a.confidence),
        "edit_p": e.ok(),
        "sec_p": s.ok(),
    })
    .to_string()
}

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
    let lines = raw.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'));
    let prompts: Vec<Item> = if path.ends_with(".jsonl") {
        lines.map(serde_json::from_str).collect::<Result<_, _>>().context("parse jsonl")?
    } else {
        lines
            .enumerate()
            .map(|(i, l)| {
                let (category, prompt) = match l.split_once('\t') {
                    Some((cat, p)) => (cat.trim().to_string(), p.trim().to_string()),
                    None => ("-".to_string(), l.to_string()),
                };
                Item { id: (i + 1).to_string(), category, prompt, context: String::new() }
            })
            .collect()
    };

    // ROUTE_MODE=code dry-runs code mode's first-turn policy (kind column = reason),
    // with code mode's own cutoffs.
    let code_mode = std::env::var("ROUTE_MODE").as_deref() == Ok("code");
    // ROUTE_SIGNALS=1 (code mode) adds a `signals` column after kind: every judgment
    // as JSON, even when a keyword decided the route, so policies can be replayed
    // offline without calling Jev again.
    let signals = code_mode && std::env::var("ROUTE_SIGNALS").is_ok();
    let (threshold, strong) = if code_mode {
        (1.0, env_or("CODE_STRONG_THRESHOLD", CODE_STRONG_THRESHOLD))
    } else {
        (threshold, strong)
    };
    let router = Arc::new(Router::new(TypeSafe::from_env()?, threshold).with_strong_threshold(strong));
    let sem = Arc::new(Semaphore::new(CONCURRENCY));
    let mut tasks = JoinSet::new();
    for (i, Item { id, category: cat, prompt, context }) in prompts.into_iter().enumerate() {
        let (router, sem) = (router.clone(), sem.clone());
        let (code_mode, signals) = (code_mode, signals);
        tasks.spawn(async move {
            let _permit = sem.acquire_owned().await;
            let d = if code_mode {
                let w = router.code_want(&prompt, &context, 1).await;
                let mut kind = w.reason.clone();
                if signals {
                    kind = format!("{kind}\t{}", code_signals(&router, &prompt, &context).await);
                }
                Ok(Decision { model: TIERS[w.tier], kind, confidence: 0.0, reason: "code" })
            } else {
                router.route(&prompt, &context).await
            };
            (i, id, cat, prompt, d)
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
    let header = if signals { "kind\tsignals" } else { "kind" };
    println!("id\tcategory\t{header}\tconfidence\tmodel\treason\tprompt");
    for (_, id, cat, prompt, d) in &rows {
        let prompt = prompt.replace('\n', "\\n").replace('\t', " ");
        match d {
            Ok(d) => {
                println!("{}\t{}\t{}\t{:.2}\t{}\t{}\t{}", id, cat, d.kind, d.confidence, d.model, d.reason, prompt);
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
