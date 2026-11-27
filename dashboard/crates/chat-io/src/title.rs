//! A session title written by a model after the first reply, instead of the first line of
//! the prompt. One small, isolated call to Claude Haiku through the user's own `claude` CLI:
//! no tools, no MCP servers, no settings or project memory, nothing saved, a neutral working
//! folder and a time limit. Any failure leaves the title unset and the UI keeps its fallback.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// The cheap model the title is asked of.
pub const TITLE_MODEL: &str = "claude-haiku-5-5";
/// Longest title kept, in characters.
const MAX_CHARS: usize = 60;
/// How much of the prompt and the reply is sent.
const PROMPT_CHARS: usize = 2_000;
const REPLY_CHARS: usize = 1_000;
/// The call is abandoned after this long.
const LIMIT: Duration = Duration::from_secs(45);

const SYSTEM: &str = "You name coding chat sessions. Reply with only a title of 3 to 6 words \
in sentence case that says what the user wants done: no quotes, no trailing punctuation, no \
preamble, no markdown. Write it in the language the user wrote in.";

fn head(text: &str, chars: usize) -> String {
    let trimmed = text.trim();
    let mut out: String = trimmed.chars().take(chars).collect();
    if out.len() < trimmed.len() {
        out.push('…');
    }
    out
}

/// What the model is asked: the first prompt and the start of the first reply.
pub fn title_request(prompt: &str, reply: &str) -> String {
    let mut text = format!(
        "Write a title for this coding chat.\n\nFirst message from the user:\n\"\"\"\n{}\n\"\"\"\n",
        head(prompt, PROMPT_CHARS)
    );
    if !reply.trim().is_empty() {
        text.push_str("\nStart of the assistant's reply:\n\"\"\"\n");
        text.push_str(&head(reply, REPLY_CHARS));
        text.push_str("\n\"\"\"\n");
    }
    text
}

/// The title from the model's answer: its first non-empty line without a `Title:` label,
/// markdown marks, quotes or a trailing full stop. `None` when nothing usable is left or the
/// answer is far too long to be a title (the model ignored the instructions).
pub fn clean_title(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line
        .strip_prefix("Title:")
        .or_else(|| line.strip_prefix("title:"))
        .unwrap_or(line);
    let line = line.trim_matches(|c: char| {
        c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '*' | '#' | '“' | '”' | '‘' | '’')
    });
    let line = line.trim_end_matches(['.', '!', ':', ';', ',']).trim();
    if line.is_empty() || line.chars().count() > MAX_CHARS * 2 {
        return None;
    }
    if line.chars().count() <= MAX_CHARS {
        return Some(line.to_owned());
    }
    let cut: String = line.chars().take(MAX_CHARS).collect();
    let cut = cut.rfind(' ').map_or(cut.as_str(), |at| &cut[..at]);
    Some(format!(
        "{}…",
        cut.trim_end_matches([',', ';', ':']).trim_end()
    ))
}

/// Asks the model for a title. `cwd` is where the CLI runs (a neutral folder, so no project
/// instructions or hooks apply). `None` on any failure: CLI missing or logged out, timeout,
/// a non-zero exit or an unusable answer.
pub async fn generate_title(bin: &str, cwd: &Path, prompt: &str, reply: &str) -> Option<String> {
    let mut child = tokio::process::Command::new(bin)
        .args([
            "-p",
            "--model",
            TITLE_MODEL,
            "--output-format",
            "text",
            "--tools",
            "",
            "--strict-mcp-config",
            "--setting-sources",
            "",
            "--no-session-persistence",
            "--system-prompt",
            SYSTEM,
        ])
        .current_dir(cwd)
        // Started from inside another Claude Code session, the CLI would think it is nested.
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let request = title_request(prompt, reply);
    let run = async move {
        stdin.write_all(request.as_bytes()).await.ok()?;
        drop(stdin);
        child.wait_with_output().await.ok()
    };
    let output = tokio::time::timeout(LIMIT, run).await.ok()??;
    if !output.status.success() {
        return None;
    }
    clean_title(&String::from_utf8_lossy(&output.stdout))
}

/// The `claude` binary: `CLAUDE_BIN` when set (as the engine does), else `claude` on PATH.
pub fn claude_bin() -> String {
    std::env::var("CLAUDE_BIN")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| String::from("claude"))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_cleaned_into_titles() {
        for (raw, want) in [
            (
                "Fix dashboard usage cache invalidation\n",
                Some("Fix dashboard usage cache invalidation"),
            ),
            (
                "\n  \"Add dark mode toggle.\"  ",
                Some("Add dark mode toggle"),
            ),
            (
                "Title: **Speed up the test suite**",
                Some("Speed up the test suite"),
            ),
            (
                "# Rename the router crate\nmore text",
                Some("Rename the router crate"),
            ),
            ("“Perbaiki bug login”", Some("Perbaiki bug login")),
            ("", None),
            ("  \n \"\" ", None),
        ] {
            assert_eq!(clean_title(raw).as_deref(), want, "{raw:?}");
        }
    }

    #[test]
    fn long_answers_are_cut_at_a_word_or_rejected() {
        let long = "Investigate why the usage dashboard shows stale numbers after a manual refresh";
        let got = clean_title(long).unwrap();
        assert!(
            got.ends_with('…') && got.chars().count() <= MAX_CHARS + 1,
            "{got}"
        );
        assert!(!got.contains("refre"), "cut at a word: {got}");
        assert_eq!(clean_title(&"word ".repeat(40)), None);
    }

    #[test]
    fn the_request_carries_a_bounded_prompt_and_reply() {
        let req = title_request(&"x".repeat(5_000), "Sure, here is the plan");
        assert!(req.contains(&format!("{}…", "x".repeat(PROMPT_CHARS))));
        assert!(req.contains("Start of the assistant's reply"));
        assert!(!title_request("fix it", "  ").contains("assistant's reply"));
    }

    #[test]
    fn a_missing_cli_gives_no_title() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let got = rt.block_on(generate_title(
            "/nonexistent/claude-for-test",
            &std::env::temp_dir(),
            "fix it",
            "",
        ));
        assert_eq!(got, None);
    }

    #[cfg(unix)]
    #[test]
    fn a_working_cli_answer_becomes_the_title() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("claude");
        std::fs::write(
            &bin,
            "#!/bin/sh\ncat >/dev/null\necho '\"Fix the flaky watcher test.\"'\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let got = rt.block_on(generate_title(
            bin.to_str().unwrap(),
            dir.path(),
            "the watcher test flakes",
            "",
        ));
        assert_eq!(got.as_deref(), Some("Fix the flaky watcher test"));
    }
}
