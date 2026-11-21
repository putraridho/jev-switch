//! Wires the gateway: reads the environment, builds the shared state and serves the routes.

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::middleware;
use axum::routing::{any, get, post};
use axum::Router as Http;
use tokio::sync::{Mutex, Semaphore};

use crate::catalog::domain::model_table::MODEL_STRONG;
use crate::gateway::application::app_state::{App, Orch};
use crate::gateway::application::session_state::Sessions;
use crate::gateway::domain::model_choice::ALIAS;
use crate::judgment::application::judge::Router;
use crate::judgment::domain::thresholds::{CODE_STRONG_THRESHOLD, ORCH_TRIGGER_P, TOP_THRESHOLD};
use crate::judgment::infrastructure::typesafe_http::TypeSafe;
use crate::shared::infrastructure::jsonl_log as log;

use super::check_handlers::{orchestrate_check, worker_check};
use super::guard::{guard_request, validate_bind_addresses, GatewayGuard};
use super::proxy_handler::proxy;

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}


pub async fn run() -> Result<()> {
    if let Some(a) = std::env::args().nth(1) {
        match a.as_str() {
            "-V" | "--version" => {
                println!("jev-gateway {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {
                println!("jev-gateway: local gateway that adds the \"{ALIAS}\" model to Claude Code. Start it with jev-claude.");
                return Ok(());
            }
        }
    }
    let addr = std::env::var("JEV_GATEWAY_ADDR").unwrap_or_else(|_| "127.0.0.1:18780".into());
    let log_path = log::path_from_env("JEV_GATEWAY_LOG", "gateway.jsonl");
    let usage_path = log::path_from_env("JEV_USAGE_LOG", "usage.jsonl");
    let addresses: Vec<_> = tokio::net::lookup_host(&addr).await.with_context(|| format!("resolve {addr}"))?.collect();
    anyhow::ensure!(!addresses.is_empty(), "no addresses found for {addr}");
    let allow_remote = std::env::var("JEV_GATEWAY_ALLOW_REMOTE").is_ok_and(|value| value == "1");
    validate_bind_addresses(&addresses, allow_remote)?;
    let listener = tokio::net::TcpListener::bind(addresses.as_slice()).await.with_context(|| format!("bind {addr}"))?;
    let loopback = listener.local_addr()?.ip().is_loopback();
    let max_inflight = match std::env::var("JEV_GATEWAY_MAX_INFLIGHT") {
        Ok(value) => value.parse::<usize>().context("JEV_GATEWAY_MAX_INFLIGHT must be a positive integer")?,
        Err(std::env::VarError::NotPresent) => 32,
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(max_inflight > 0 && max_inflight <= Semaphore::MAX_PERMITS, "JEV_GATEWAY_MAX_INFLIGHT is out of range");
    let app = Arc::new(App {
        router: Router::new(TypeSafe::from_env()?, 1.0).with_jev_timeout_from(&crate::shared::infrastructure::system_host::SystemHost).with_strong_threshold(env_f64("CODE_STRONG_THRESHOLD", CODE_STRONG_THRESHOLD))
            // Fable for the hardest Opus-bound turns; JEV_FABLE=off keeps tiers 0-2.
            .with_worker_cheap(std::env::var("WORKER_CHEAP_P").ok().and_then(|v| v.parse().ok()))
            .with_top_threshold(std::env::var("JEV_FABLE").map_or(true, |v| v != "off").then(|| env_f64("FABLE_THRESHOLD", TOP_THRESHOLD))),
        http: reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()?,
        upstream: std::env::var("JEV_GATEWAY_UPSTREAM")
            .unwrap_or_else(|_| "https://api.anthropic.com".into())
            .trim_end_matches('/')
            .to_string(),
        downgrade_after: env_f64("DOWNGRADE_AFTER", 2.0) as u32,
        sessions: Mutex::new(Sessions::new(Arc::new(crate::shared::infrastructure::system_host::SystemHost))),
        log_path,
        usage_path,
        turns_path: log::path_from_env("JEV_TURNS_LOG", "turns.jsonl"),
        orch: Orch {
            enabled: std::env::var("JEV_ORCH").map_or(true, |v| v != "off"),
            cutoff: env_f64("ORCH_TRIGGER_P", ORCH_TRIGGER_P),
            // Leaked once at startup: routing hands out &'static model names.
            planner: Box::leak(std::env::var("ORCH_PLANNER").unwrap_or_else(|_| MODEL_STRONG.into()).into_boxed_str()),
            planner_pinned: std::env::var("ORCH_PLANNER").is_ok(),
            require_opus: std::env::var("ORCH_REQUIRE_OPUS").map_or(true, |v| v != "0" && v != "off"),
            log_path: log::path_from_env("JEV_ORCH_LOG", "orch.jsonl"),
        },
    });
    let http = Http::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/jev/orchestrate", post(orchestrate_check))
        .route("/jev/worker", post(worker_check))
        .fallback(any(proxy))
        .layer(middleware::from_fn_with_state(GatewayGuard { loopback, permits: Arc::new(Semaphore::new(max_inflight)) }, guard_request))
        .with_state(app);
    eprintln!("jev-gateway {} listening on http://{addr}", env!("CARGO_PKG_VERSION"));
    axum::serve(listener, http).await?;
    Ok(())
}
