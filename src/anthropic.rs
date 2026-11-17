//! Claude client that runs turns through Claude Code (`claude -p`), so calls are
//! covered by a Claude subscription instead of an API key. Each call is stateless:
//! the full message history is sent every time.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Cheap model for confidently-simple turns.
pub const MODEL_CHEAP: &str = "claude-haiku-5-5";
/// Mid model for everyday coding, and for anything simple-but-uncertain or unclear.
pub const MODEL_MID: &str = "claude-sonnet-5-5";
/// Strong model for planning, tradeoffs, hard debugging, and security-sensitive work.
pub const MODEL_STRONG: &str = "claude-opus-5-5";
/// Top model for the hardest turns, used by the gateway only (`Router::with_top_threshold`).
pub const MODEL_TOP: &str = "claude-fable-5-2";

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
        if is_limit_message(&parsed.result) {
            bail!("Claude usage limit reached: {}", parsed.result.trim());
        }
        Ok((parsed.result, parsed.usage))
    }
}

/// How far after "limit" the word "resets" may come and still be the limit's reset time.
const RESETS_WITHIN: usize = 30;

/// Most words that may come before "limit" in a limit notice ("you've hit your Opus
/// weekly limit" has five); longer lead-ins are ordinary sentences.
const MAX_WORDS_BEFORE_LIMIT: usize = 6;

/// Wording of a used-up plan or usage limit, as Claude Code and the other CLIs say it
/// ("You've hit your session limit", "5-hour limit reached ∙ resets 3pm", "Opus weekly
/// limit reached", "Claude AI usage limit reached|<epoch>", "Approaching usage limit").
/// `lower` must be lowercased with typographic apostrophes already replaced. This is
/// the ONE phrase source: the shared error classifier (`backend::classify_common`) and
/// [`is_limit_message`] both ask it. Phrases only, never a bare "limit" or "resets":
/// "limit the query to 10 rows" and "the resets handler" are ordinary text.
pub fn is_limit_phrase(lower: &str) -> bool {
    let after = |needle: &str| lower.split_once(needle).map(|(_, rest)| rest);
    lower.contains("usage limit")
        || lower.contains("limit reached")
        || (lower.contains("hit your") && lower.contains("limit"))
        || after("reached your").is_some_and(|rest| rest.contains("limit"))
        // "<x> limit ∙ resets <when>": the reset comes right after the limit, in the same breath.
        || after("limit").is_some_and(|rest| {
            rest.find("resets").is_some_and(|i| i <= RESETS_WITHIN && !rest[..i].contains(['.', ';']))
        })
}

/// Claude Code reports a used-up plan limit as the answer text, with no error flag.
/// Shown as an answer it reads like the model replying; it must be treated as a
/// usage-limit error. Only the opening sentence is checked, and "limit" must come
/// early in it, so a real answer that merely mentions limits isn't caught.
pub fn is_limit_message(text: &str) -> bool {
    let head: String = text.trim_start().chars().take(120).collect::<String>().to_lowercase();
    let head = head.replace('\u{2019}', "'");
    let sentence = head.split(['.', ';', '\n', '!', '?']).next().unwrap_or("");
    let lead_words = sentence.split_once("limit").map_or(usize::MAX, |(lead, _)| lead.split_whitespace().count());
    lead_words <= MAX_WORDS_BEFORE_LIMIT && is_limit_phrase(sentence)
}

#[cfg(test)]
mod tests {
    use super::{is_limit_message, is_limit_phrase};

    #[test]
    fn real_claude_limit_notices_are_recognised() {
        for text in [
            "5-hour limit reached \u{2219} resets 3pm",
            "Weekly limit reached \u{2219} resets Nov 24",
            "Opus weekly limit reached",
            "You've hit your session limit \u{b7} resets 5pm (Asia/Jakarta)",
            "Claude AI usage limit reached|1794853451",
            "Approaching usage limit",
            "Limit reached",
            "You have reached your weekly limit",
        ] {
            assert!(is_limit_message(text), "{text:?}");
        }
    }

    #[test]
    fn ordinary_text_mentioning_limits_is_not_a_limit_phrase() {
        for text in [
            "the resets handler",
            "rate limit middleware",
            "limit the query to 10 rows",
            "quota calculation function",
            "the limit is 10; the timer resets later",
            "when the timer resets the counter goes to zero",
        ] {
            assert!(!is_limit_phrase(text), "{text:?}");
            assert!(!is_limit_message(text), "{text:?}");
        }
        assert!(!is_limit_message("I read the whole module and the limit reached in the loop is fine."));
    }

    #[test]
    fn plan_limit_text_is_recognised_but_answers_about_limits_are_not() {
        assert!(is_limit_message("You've hit your session limit · resets 5pm (Asia/Jakarta)"));
        assert!(is_limit_message("You\u{2019}ve hit your weekly limit"));
        assert!(is_limit_message("  Claude AI usage limit reached|1794853451"));
        assert!(!is_limit_message("The rate limit is 50 requests per minute; you've hit your limit when it returns 429."));
        assert!(!is_limit_message("Fixed the off-by-one in the pagination limit."));
        assert!(!is_limit_message(""));
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
