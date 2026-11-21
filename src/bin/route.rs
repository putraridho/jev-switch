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
//!
//! `ROUTE_MODE=provider` measures the multi-provider router's Jev judgment instead of
//! the Claude tiers: per prompt it asks `provider_want` once and prints the task type
//! with its confidence, the yes/no signals, and the model `routing::decide` would pick
//! on a first turn for a user with Claude and Codex (Plus models) usable, or `none`.
//! `ROUTE_JSON=1` prints JSONL (one object per prompt, `"failed": true` when Jev gave
//! no answer) for `scripts/task_type_accuracy.py`. `ROUTE_CODEX_MODELS=id,id` sets the
//! usable Codex models (default: every catalog Codex model except the very-hard-only
//! Astra, which the Plus plan does not include) and `ROUTE_CREDITS=1` allows
//! pay-as-you-go models such as Fable.
//!
//!   ROUTE_MODE=provider ROUTE_JSON=1 cargo run --release --bin route prompts.jsonl > out.jsonl

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use jev_router::catalog::domain::model_table::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use jev_router::shared::ProviderId;
use jev_router::catalog::domain::model_table as catalog;
use jev_router::catalog::domain::model_table::{Allowances, Overrides, Usable};
use jev_router::routing::domain::decide as routing;
use jev_router::routing::domain::decide::{Pin, RouteInput, RoutingState, Signals};
use jev_router::judgment::application::judge::Router;
use jev_router::judgment::domain::security::security_level;
use jev_router::judgment::domain::thresholds::CODE_STRONG_THRESHOLD;
use jev_router::judgment::domain::want::Decision;

const TIERS: [&str; 3] = [MODEL_CHEAP, MODEL_MID, MODEL_STRONG];
use jev_router::judgment::infrastructure::typesafe_http::TypeSafe;

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

/// What `ROUTE_MODE=provider` decides with: a user with Claude and Codex usable.
struct ProviderSetup {
    usable: Usable,
    allow: Allowances,
    overrides: Overrides,
}

impl ProviderSetup {
    fn from_env() -> Self {
        let codex: Vec<String> = match std::env::var("ROUTE_CODEX_MODELS") {
            Ok(list) => list.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            // Plus does not include the very-hard-only Astra.
            Err(_) => catalog::models()
                .iter()
                .filter(|m| m.provider == ProviderId::Codex && !m.very_hard_only)
                .map(|m| m.id.to_string())
                .collect(),
        };
        let claude = catalog::models().iter().filter(|m| m.provider == ProviderId::Claude).map(|m| m.id.to_string());
        let usable = Usable::new(
            claude.map(|id| (ProviderId::Claude, id)).chain(codex.into_iter().map(|id| (ProviderId::Codex, id))),
        );
        let credits = std::env::var("ROUTE_CREDITS").is_ok_and(|v| !v.is_empty() && v != "0");
        Self { usable, allow: Allowances { credits, per_token: false }, overrides: jev_router::catalog::application::chain_overrides::from_env() }
    }
}

/// One prompt in provider mode: the signals as a JSON object (with the model that
/// `routing::decide` picks) or `None` when Jev gave no answer. The decision is the
/// first turn of a fresh session, so no stickiness applies.
async fn provider_row(router: &Router, setup: &ProviderSetup, prompt: &str, context: &str) -> Option<(serde_json::Value, &'static str)> {
    // Orchestration on: the tool shows every signal, the sequence veto included.
    let w = router.provider_want(prompt, context, None, true).await?;
    let signals = Signals::from(&w);
    let plan = |_: ProviderId| None;
    let input = RouteInput {
        signals: Some(&signals),
        prompt_words: prompt.split_whitespace().count(),
        pin: &Pin::Auto,
        usable: &setup.usable,
        allow: &setup.allow,
        overrides: &setup.overrides,
        context_tokens: 0,
        plan_utilization: &plan,
        now_ms: 0,
    };
    let model = routing::decide(&mut RoutingState::default(), &input).map(|d| d.spec.id).unwrap_or("none");
    let row = serde_json::json!({
        "task": w.task.as_str(),
        "task_confidence": w.task_confidence,
        "security_p": w.security_p,
        "very_hard_p": w.very_hard_p,
        "continues_p": w.continues_p,
        "multi_part_p": w.orchestration.multi_part_p,
        "model": model,
    });
    Some((row, model))
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

    // Sonnet matched Opus on every sampled prompt below this; see docs/guide.md.
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
    let provider_mode = std::env::var("ROUTE_MODE").as_deref() == Ok("provider");
    let as_json = std::env::var("ROUTE_JSON").is_ok_and(|v| !v.is_empty() && v != "0");
    let setup = Arc::new(ProviderSetup::from_env());
    let router = Arc::new(Router::new(TypeSafe::from_env()?, threshold).with_jev_timeout_from(&jev_router::shared::infrastructure::system_host::SystemHost).with_strong_threshold(strong));
    let sem = Arc::new(Semaphore::new(CONCURRENCY));
    let mut tasks = JoinSet::new();
    for (i, Item { id, category: cat, prompt, context }) in prompts.into_iter().enumerate() {
        let (router, sem, setup) = (router.clone(), sem.clone(), setup.clone());
        let (code_mode, signals) = (code_mode, signals);
        tasks.spawn(async move {
            let _permit = sem.acquire_owned().await;
            let mut extra = None;
            let d = if provider_mode {
                match provider_row(&router, &setup, &prompt, &context).await {
                    Some((row, model)) => {
                        let conf = row["task_confidence"].as_f64().unwrap_or(0.0);
                        let kind = row["task"].as_str().unwrap_or("").to_string();
                        extra = Some(row);
                        Ok(Decision { model, kind, confidence: conf, reason: "provider" })
                    }
                    None => Err(anyhow::anyhow!("Jev gave no usable answer (timeout, error or unreadable)")),
                }
            } else if code_mode {
                let w = router.code_want(&prompt, &context, 1).await;
                let mut kind = w.reason.clone();
                if signals {
                    kind = format!("{kind}\t{}", code_signals(&router, &prompt, &context).await);
                }
                Ok(Decision { model: TIERS[w.tier], kind, confidence: 0.0, reason: "code" })
            } else {
                router.route(&prompt, &context).await
            };
            (i, id, cat, prompt, d, extra)
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
    if provider_mode {
        return print_provider(&rows, as_json);
    }
    let header = if signals { "kind\tsignals" } else { "kind" };
    println!("id\tcategory\t{header}\tconfidence\tmodel\treason\tprompt");
    for (_, id, cat, prompt, d, _) in &rows {
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

type Row = (usize, String, String, String, Result<Decision>, Option<serde_json::Value>);

/// Provider-mode output: TSV or JSONL on stdout, failure count and task histogram on
/// stderr. A prompt Jev did not answer is never dropped: JSONL carries it as
/// `"failed": true`, TSV as a `FAILED` row.
fn print_provider(rows: &[Row], as_json: bool) -> Result<()> {
    let cols = ["task", "task_confidence", "security_p", "very_hard_p", "continues_p", "multi_part_p", "model"];
    if !as_json {
        println!("id\tcategory\t{}\tprompt", cols.join("\t"));
    }
    let (mut failed, mut tasks): (usize, BTreeMap<String, usize>) = (0, BTreeMap::new());
    for (_, id, cat, prompt, d, extra) in rows {
        let Some(row) = extra.as_ref().filter(|_| d.is_ok()) else {
            failed += 1;
            if let Err(e) = d {
                eprintln!("  error on {:?}: {:#}", prompt, e);
            }
            if as_json {
                println!("{}", serde_json::json!({"id": id, "category": cat, "failed": true}));
            } else {
                println!("{id}\t{cat}\tFAILED\t\t\t\t\t\t\t{}", prompt.replace('\n', "\\n").replace('\t', " "));
            }
            continue;
        };
        *tasks.entry(row["task"].as_str().unwrap_or("").to_string()).or_default() += 1;
        if as_json {
            let mut obj = serde_json::json!({"id": id, "category": cat, "prompt": prompt});
            obj.as_object_mut().expect("object").extend(row.as_object().expect("object").clone());
            println!("{obj}");
        } else {
            let num = |k: &str| format!("{:.2}", row[k].as_f64().unwrap_or(0.0));
            println!(
                "{id}\t{cat}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                row["task"].as_str().unwrap_or(""),
                num("task_confidence"),
                num("security_p"),
                num("very_hard_p"),
                num("continues_p"),
                num("multi_part_p"),
                row["model"].as_str().unwrap_or(""),
                prompt.replace('\n', "\\n").replace('\t', " ")
            );
        }
    }
    eprintln!("\nprovider mode: {} prompts, {} failed (Jev gave no answer)", rows.len(), failed);
    for (task, n) in &tasks {
        eprintln!("  {task:<12} {n}");
    }
    Ok(())
}
