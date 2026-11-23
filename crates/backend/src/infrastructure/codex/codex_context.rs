//! How full Codex's context window is after a turn, read from the thread's rollout file.
//!
//! `codex exec --json` reports one `usage` per turn: the sum over every model request the
//! turn made. An agent turn re-sends the conversation on each tool call, so forty calls on
//! a 50K conversation report about 2M input tokens, which is no measure of the context.
//! The rollout (`$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<time>-<thread id>.jsonl`) records
//! a `token_count` event after each request, whose `last_token_usage` is that request
//! alone: the size the context window actually holds.

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// `{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{..}}}}`.
/// Only the fields read here; other lines and fields are ignored.
#[derive(Deserialize)]
struct RolloutLine {
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    info: Option<Info>,
}

#[derive(Deserialize)]
struct Info {
    last_token_usage: Option<RequestUsage>,
    /// The window of the model that served this request, as Codex itself reports it.
    model_context_window: Option<u64>,
}

#[derive(Deserialize)]
struct RequestUsage {
    /// Includes the cached share, the same scale as `Usage::context_tokens`.
    input_tokens: u64,
}

/// What one `token_count` line says about the request it follows.
struct RequestFacts {
    input_tokens: u64,
    /// The model's window, when Codex printed one.
    window_tokens: Option<u64>,
}

/// The last request on a rollout line, when the line is a `token_count`.
fn request_facts(line: &str) -> Option<RequestFacts> {
    let info = serde_json::from_str::<RolloutLine>(line).ok()?.payload?.info?;
    let usage = info.last_token_usage?;
    (usage.input_tokens > 0).then_some(RequestFacts {
        input_tokens: usage.input_tokens,
        window_tokens: info.model_context_window.filter(|w| *w > 0),
    })
}

/// The rollout of `thread_id` under `sessions_dir` (year/month/day folders).
fn rollout_of(sessions_dir: &Path, thread_id: &str) -> Option<PathBuf> {
    let suffix = format!("-{thread_id}.jsonl");
    fn find(dir: &Path, depth: usize, suffix: &str) -> Option<PathBuf> {
        for entry in fs::read_dir(dir).ok()?.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if depth > 0 && kind.is_dir() {
                if let Some(found) = find(&entry.path(), depth - 1, suffix) {
                    return Some(found);
                }
            } else if depth == 0 && kind.is_file() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("rollout-") && name.ends_with(suffix) {
                    return Some(entry.path());
                }
            }
        }
        None
    }
    find(sessions_dir, 3, &suffix)
}

/// The latest request's facts from the thread's rollout under `sessions_dir`.
fn latest_request(sessions_dir: &Path, thread_id: &str) -> Option<RequestFacts> {
    if thread_id.is_empty() {
        return None;
    }
    let file = File::open(rollout_of(sessions_dir, thread_id)?).ok()?;
    BufReader::new(file).lines().map_while(Result::ok).filter_map(|line| request_facts(&line)).last()
}

/// The context size after the thread's latest request, from its rollout under `sessions_dir`.
pub fn context_tokens_in(sessions_dir: &Path, thread_id: &str) -> Option<u64> {
    latest_request(sessions_dir, thread_id).map(|facts| facts.input_tokens)
}

/// The window Codex reported for the model that served the thread's latest request.
pub fn window_tokens_in(sessions_dir: &Path, thread_id: &str) -> Option<u64> {
    latest_request(sessions_dir, thread_id).and_then(|facts| facts.window_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01a1f9f1-a69b-73e0-8fb5-b2ed1db076c2";

    fn token_line(total: u64, last: u64) -> String {
        format!(
            r#"{{"type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{total}}},"last_token_usage":{{"input_tokens":{last},"cached_input_tokens":1}},"model_context_window":475000}}}}}}"#
        )
    }

    #[test]
    fn reads_the_last_request_not_the_turn_total() {
        let root = jev_testkit::scratch_dir::unique_scratch_dir("codex-context", "t");
        let day = root.join("2026/11/21");
        fs::create_dir_all(&day).unwrap();
        let rollout = format!(
            "{}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":null}}}}\nnot json\n{}\n{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\"}}}}\n",
            token_line(827_635, 50_389),
            token_line(878_394, 50_759)
        );
        fs::write(day.join(format!("rollout-2026-11-21T00-12-09-{ID}.jsonl")), rollout).unwrap();
        fs::write(day.join("rollout-2026-11-21T00-00-00-other.jsonl"), token_line(9, 9)).unwrap();
        assert_eq!(context_tokens_in(&root, ID), Some(50_759));
        assert_eq!(window_tokens_in(&root, ID), Some(475_000), "the window of the last request");
        assert_eq!(context_tokens_in(&root, "missing"), None);
        assert_eq!(window_tokens_in(&root, "missing"), None);
        assert_eq!(context_tokens_in(&root, ""), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_rollout_without_token_counts_says_nothing() {
        let root = jev_testkit::scratch_dir::unique_scratch_dir("codex-context", "t");
        let day = root.join("2026/11/21");
        fs::create_dir_all(&day).unwrap();
        fs::write(day.join(format!("rollout-x-{ID}.jsonl")), "{\"type\":\"session_meta\"}\n").unwrap();
        assert_eq!(context_tokens_in(&root, ID), None);
        fs::remove_dir_all(root).unwrap();
    }
}
