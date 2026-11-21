//! The delegate_mcp process: command line, stdin reader, one task per request and a single
//! writer task, so concurrent calls never interleave their output lines.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, OnceCell};
use tokio::task::JoinSet;

use super::json_rpc::{handle, Server};
use super::lazy_delegator::Lazy;
use crate::orchestration::infrastructure::worker_file::has_leftovers;
use crate::shared::infrastructure::jsonl_log as log;

/// `--dir` and `--planner` from the command line.
pub(super) fn parse_args(args: impl IntoIterator<Item = String>) -> Result<(Option<PathBuf>, String)> {
    let (mut dir, mut planner) = (None, String::new());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dir" => dir = Some(PathBuf::from(it.next().context("--dir needs a value")?)),
            "--planner" => planner = it.next().context("--planner needs a value")?,
            other => anyhow::bail!("unknown option {other} (usage: delegate_mcp [--dir <dir>] [--planner <provider:model>])"),
        }
    }
    Ok((dir, planner))
}

pub async fn run() -> Result<()> {
    let (dir, planner) = parse_args(std::env::args().skip(1))?;
    let dir = match dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };
    let lazy = Arc::new(Lazy { dir, planner, cell: OnceCell::new() });
    // Workers an earlier run left behind are cleaned up at startup, but providers are only
    // probed for that when there is something to clean: a session that never delegates stays free.
    if log::state_dir().is_some_and(|s| has_leftovers(&s)) {
        let warm = lazy.clone();
        tokio::spawn(async move {
            let _ = warm.delegator().await;
        });
    }
    let server = Server { api: lazy };

    // Replies go through one writer task so concurrent calls never interleave their lines.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut out = tokio::io::stdout();
        while let Some(line) = rx.recv().await {
            if out.write_all(line.as_bytes()).await.is_err() || out.write_all(b"\n").await.is_err() || out.flush().await.is_err() {
                break;
            }
        }
    });

    // One task per request, so a long `delegate` does not block the next one.
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut in_flight = JoinSet::new();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let (server, tx) = (server.clone(), tx.clone());
        in_flight.spawn(async move {
            if let Some(reply) = handle(&line, &server).await {
                let _ = tx.send(reply);
            }
        });
    }
    // stdin closed: answer what is still running, then stop. (A client that is really gone
    // also kills this process, and the worker CLIs die with it: they are killed on drop.)
    while in_flight.join_next().await.is_some() {}
    drop(tx);
    let _ = writer.await;
    Ok(())
}
