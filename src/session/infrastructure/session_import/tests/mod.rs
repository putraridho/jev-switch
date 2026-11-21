//! Tests for importing Claude Code and Codex histories.

use std::path::PathBuf;

use super::claude::claude_turns;
use super::codex::codex_turns;
use super::*;
use crate::backend::ProviderId;
use crate::session::domain::tool_trail;

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("jev-imp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const CLAUDE: &str = r#"{"type":"summary","summary":"x"}
{"type":"user","message":{"role":"user","content":"fix the parser"}}
{"type":"user","isMeta":true,"message":{"role":"user","content":"Caveat: meta"}}
{"type":"assistant","message":{"model":"claude-sonnet-5-5","content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"Looking."},{"type":"tool_use","id":"1","name":"Edit","input":{"file_path":"/p/src/parser.rs","old_string":"a"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"1","content":"ok"}]}}
{"type":"assistant","message":{"model":"claude-sonnet-5-5","content":[{"type":"text","text":"Fixed it."}]}}
{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"now add tests"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"2","name":"Bash","input":{"command":"cargo test"}},{"type":"text","text":"Tests pass."}]}}
this line is not json
"#;

const CODEX: &str = r#"{"type":"session_meta","payload":{"id":"x","cwd":"/p"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>..</environment_context>"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"list the files"}]}}
{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"ls -la\"]}"}}
{"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","arguments":"{\"input\":\"*** Begin Patch\\n*** Update File: src/a.rs\\n@@\"}"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Here they are."}]}}
{"type":"event_msg","payload":{"type":"token_count"}}
"#;

fn prompts(turns: &[TurnRecord]) -> Vec<&str> {
    turns.iter().map(|t| t.prompt.as_str()).collect()
}

#[test]
fn claude_code_history_becomes_turns_with_tools_and_the_final_answer() {
    let turns = claude_turns(CLAUDE);
    assert_eq!(prompts(&turns), ["fix the parser", "now add tests"]);
    assert_eq!(turns[0].reply, "Fixed it.");
    assert_eq!(turns[0].model, "claude-sonnet-5-5");
    assert_eq!(turns[0].tools, ["Edit /p/src/parser.rs"]);
    assert_eq!(turns[1].tools, ["Bash cargo test"]);
    assert_eq!(turns[1].reply, "Tests pass.");
    assert!(turns.iter().all(|t| t.provider == ProviderId::Claude && t.ok));
    // Tools in jev's format are understood by the hand-off.
    assert_eq!(tool_trail::files_touched(&turns[0].tools), ["/p/src/parser.rs"]);
    assert_eq!(tool_trail::commands_run(&turns[1].tools), ["cargo test"]);
}

#[test]
fn codex_history_skips_injected_context_and_names_shell_and_patch_calls() {
    let turns = codex_turns(CODEX);
    assert_eq!(prompts(&turns), ["list the files"]);
    assert_eq!(turns[0].tools, ["bash ls -la", "apply_patch src/a.rs"]);
    assert_eq!(turns[0].reply, "Here they are.");
    assert_eq!(turns[0].provider, ProviderId::Codex);
    assert_eq!(tool_trail::files_touched(&turns[0].tools), ["src/a.rs"]);
}

#[test]
fn discovery_finds_this_projects_histories_in_both_tools() {
    let home = temp("discover");
    let project = home.join("work").join("my.app");
    std::fs::create_dir_all(&project).unwrap();
    let claude = claude_project_dir(&home, &project);
    assert!(claude.to_string_lossy().ends_with("-work-my-app"), "{claude:?}");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(claude.join("a.jsonl"), CLAUDE).unwrap();
    std::fs::write(claude.join("notes.txt"), "no").unwrap();
    let day = home.join(".codex/sessions/2026/11/17");
    std::fs::create_dir_all(&day).unwrap();
    let mine = CODEX.replace("\"/p\"", &serde_json::to_string(&session_file::project_key(&project)).unwrap());
    std::fs::write(day.join("rollout-1.jsonl"), mine).unwrap();
    std::fs::write(day.join("rollout-2.jsonl"), CODEX).unwrap(); // another project's
    let found = discover(&home, &project);
    let mut sources: Vec<Source> = found.iter().map(|f| f.source).collect();
    sources.sort_by_key(|s| s.label());
    assert_eq!(sources, [Source::ClaudeCode, Source::Codex]);
    let codex = found.iter().find(|f| f.source == Source::Codex).unwrap();
    assert!(codex.path.ends_with("rollout-1.jsonl"));
    assert_eq!(load(codex).unwrap().len(), 1);
}

#[test]
fn an_empty_history_is_an_error_and_a_loaded_one_resumes() {
    let dir = temp("load");
    let empty = dir.join("e.jsonl");
    std::fs::write(&empty, "{\"type\":\"summary\"}\n").unwrap();
    let found = Found { source: Source::ClaudeCode, path: empty, modified_ms: 5 };
    assert!(load(&found).is_err());
    let saved = to_saved(claude_turns(CLAUDE), &dir, 5);
    assert_eq!(saved.sessions.turns(), 2);
    // Nobody has lived these turns, so the first provider to run is handed them.
    assert_eq!(saved.sessions.missed_turns(ProviderId::Gemini), 2);
}
