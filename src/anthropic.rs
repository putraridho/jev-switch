//! Claude client that runs turns through Claude Code (`claude -p`), so calls are
//! covered by a Claude subscription instead of an API key. Each call is stateless:
//! the full message history is sent every time.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Cheap model for confidently-simple turns.
pub const MODEL_CHEAP: &str = "claude-haiku-4-5";
/// Mid model for everyday coding, and for anything simple-but-uncertain or unclear.
pub const MODEL_MID: &str = "claude-sonnet-5-5";
/// Strong model for planning, tradeoffs, hard debugging, and security-sensitive work.
pub const MODEL_STRONG: &str = "claude-opus-5-5";

/// One turn in the conversation. This is the canonical, provider-neutral unit
/// the transcript stores and replays to whichever model handles the turn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Message { role: "user".into(), content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Message { role: "assistant".into(), content: content.into() }
    }
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    result: String,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    usage: Usage,
}

#[derive(Debug, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

pub struct Anthropic {
    bin: String,
}

impl Anthropic {
    /// Uses the `claude` binary on PATH, or `CLAUDE_BIN` if set.
    pub fn from_env() -> Result<Self> {
        let bin = std::env::var("CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
        Ok(Self { bin })
    }

    /// Send `system` + the full `messages` history to `model`. Carrying the whole
    /// history here — regardless of which model answered earlier turns — is what
    /// preserves memory across a route switch.
    ///
    /// `max_tokens` is unused: Claude Code doesn't expose an output cap.
    pub async fn complete(
        &self,
        model: &str,
        system: Option<&str>,
        messages: &[Message],
        _max_tokens: u32,
    ) -> Result<(String, Usage)> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(["-p", "--model", model, "--output-format", "json"])
            // Plain chat: no tools, no MCP, no user/project settings, no saved session.
            .args(["--tools", "", "--strict-mcp-config", "--setting-sources", ""])
            .arg("--no-session-persistence")
            // Neutral cwd so no project CLAUDE.md leaks into the context.
            .current_dir(std::env::temp_dir())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(system) = system {
            cmd.args(["--system-prompt", system]);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to run `{}` (is Claude Code installed?)", self.bin))?;
        let mut stdin = child.stdin.take().context("claude stdin")?;
        stdin.write_all(render_prompt(messages).as_bytes()).await?;
        drop(stdin);

        let out = child.wait_with_output().await.context("claude did not finish")?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let parsed: Response = serde_json::from_str(&stdout).with_context(|| {
            format!(
                "parse claude output (exit {}): {}{}",
                out.status,
                stdout.trim(),
                String::from_utf8_lossy(&out.stderr).trim()
            )
        })?;
        if parsed.is_error || !out.status.success() {
            bail!("claude {}: {}", out.status, parsed.result);
        }
        Ok((parsed.result, parsed.usage))
    }
}

/// Flatten the history into one prompt: earlier turns as a labeled transcript,
/// then the latest user message to answer.
fn render_prompt(messages: &[Message]) -> String {
    let Some((last, earlier)) = messages.split_last() else {
        return String::new();
    };
    if earlier.is_empty() {
        return last.content.clone();
    }
    let history = earlier
        .iter()
        .map(|m| {
            let who = if m.role == "assistant" { "Assistant" } else { "User" };
            format!("{}: {}", who, m.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "<conversation_so_far>\n{}\n</conversation_so_far>\n\nContinue the conversation. Reply to this latest user message:\n\n{}",
        history, last.content
    )
}
