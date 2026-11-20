//! Ask the provider CLIs how their MCP servers are doing, with their own list commands.
//!
//! `claude mcp list` connects to every server and prints one line each
//! (`name: target - ✔ Connected`). `codex mcp list --json` does not connect, but says
//! whether a server is switched off or still needs an OAuth sign-in. Nothing else is
//! claimed for Codex: an enabled server it cannot vouch for stays [`McpHealth::Unknown`].

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::mcp_discovery::McpScope;
use super::mcp_server::{McpHealth, McpServer};
use crate::shared::infrastructure::env_file;

/// Claude's check connects to every server in turn, so it is slow; past this it is dropped.
const CHECK_TIMEOUT: Duration = Duration::from_secs(45);

/// The text with escape sequences and other control characters taken out.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI (`ESC [ … final`) or a two-character escape.
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&f) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else if c == '\n' || !c.is_control() {
            out.push(c);
        }
    }
    out
}

/// The status words after a symbol such as `✔`, `✗`, `⚠` or `⏸`.
fn status_words(status: &str) -> &str {
    status.trim().trim_start_matches(|c: char| !c.is_alphanumeric()).trim()
}

/// One server line of `claude mcp list`: the name ends at the first `": "` (names such as
/// `plugin:playwright:playwright` hold colons), the status follows the last `" - "`.
fn claude_line(line: &str) -> Option<(String, McpHealth)> {
    let (name, rest) = line.split_once(": ")?;
    let (_, status) = rest.rsplit_once(" - ")?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let words = status_words(status);
    let lower = words.to_lowercase();
    let health = if lower.contains("connected") && !lower.contains("failed") {
        McpHealth::Connected
    } else if lower.contains("auth") {
        McpHealth::NeedsLogin
    } else if lower.contains("pending approval") {
        McpHealth::Failed("pending approval in claude".into())
    } else if lower.contains("disabled") {
        McpHealth::Disabled
    } else {
        McpHealth::Failed(words.to_string())
    };
    Some((name.to_string(), health))
}

/// Each server `claude mcp list` printed, with its state.
pub fn parse_claude_list(stdout: &str) -> Vec<(String, McpHealth)> {
    plain(stdout).lines().filter_map(claude_line).collect()
}

/// One entry of `codex mcp list --json`; other fields are ignored.
#[derive(Deserialize)]
struct CodexEntry {
    name: String,
    enabled: bool,
    auth_status: Option<String>,
}

/// Each server `codex mcp list --json` names: switched off, waiting for a sign-in, or
/// (since Codex does not connect) not checked.
pub fn parse_codex_json(stdout: &str) -> Vec<(String, McpHealth)> {
    let Ok(entries) = serde_json::from_str::<Vec<CodexEntry>>(stdout) else { return Vec::new() };
    entries
        .into_iter()
        .map(|e| {
            let auth = e.auth_status.unwrap_or_default();
            let health = if !e.enabled {
                McpHealth::Disabled
            } else if auth.contains("not_logged") || auth.contains("logged_out") {
                McpHealth::NeedsLogin
            } else {
                McpHealth::Unknown
            };
            (e.name, health)
        })
        .collect()
}

/// Run `program args`, killing it after `timeout`: its stdout, or why it failed.
fn run(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let shown = format!("{program} {}", args.join(" "));
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run `{shown}`: {e}"))?;
    // Read both pipes on threads so a full pipe cannot stall the child.
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_string(&mut s);
        }
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_string(&mut s);
        }
        s
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("`{shown}` took longer than {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("`{shown}`: {e}")),
        }
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if status.success() {
        Ok(stdout)
    } else {
        let said = plain(&stderr).trim().lines().last().unwrap_or("").to_string();
        Err(if said.is_empty() { format!("`{shown}` failed ({status})") } else { format!("`{shown}` failed: {said}") })
    }
}

/// What the provider's CLI says about each of its servers. Only Claude and Codex have a
/// list command; other scopes answer with nothing. Blocking: call it off the UI thread.
pub fn check(scope: McpScope) -> Result<Vec<(String, McpHealth)>, String> {
    match scope {
        McpScope::Claude => {
            let bin = env_file::lookup("CLAUDE_BIN").unwrap_or_else(|| "claude".into());
            run(&bin, &["mcp", "list"], CHECK_TIMEOUT).map(|out| parse_claude_list(&out))
        }
        McpScope::Codex => {
            let bin = env_file::lookup("CODEX_BIN").unwrap_or_else(|| "codex".into());
            run(&bin, &["mcp", "list", "--json"], CHECK_TIMEOUT).map(|out| parse_codex_json(&out))
        }
        McpScope::Orchestrate | McpScope::Project | McpScope::Gemini => Ok(Vec::new()),
    }
}

/// How long a successful check is reused: opening `/mcp` twice in a row asks once.
const REUSE_FOR: Duration = Duration::from_secs(60);

/// A provider's last successful check.
struct Kept {
    scope: McpScope,
    at: Instant,
    said: Vec<(String, McpHealth)>,
}

/// The last successful check of each provider.
static RECENT: std::sync::Mutex<Vec<Kept>> = std::sync::Mutex::new(Vec::new());

/// [`check`], reusing an answer younger than [`REUSE_FOR`]. Failures are not kept.
fn check_reusing(scope: McpScope) -> Result<Vec<(String, McpHealth)>, String> {
    let recent = || RECENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(k) = recent().iter().find(|k| k.scope == scope && k.at.elapsed() < REUSE_FOR) {
        return Ok(k.said.clone());
    }
    let said = check(scope)?;
    let mut kept = recent();
    kept.retain(|k| k.scope != scope);
    kept.push(Kept { scope, at: Instant::now(), said: said.clone() });
    Ok(said)
}

/// Drop the kept answers, so the next check asks again (after a login, say).
pub fn forget_checks() {
    RECENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
}

/// One answer of [`check_in_background`]: this provider's view of this server.
pub type Answer = (String, McpScope, McpHealth);

/// Check every provider the servers are configured under (reusing a check of the last
/// minute), each on its own thread, and
/// send one answer per server and provider as each check ends. The channel closes when
/// every check has. A failed check marks all of that provider's servers failed with its
/// reason; a server the CLI did not mention becomes [`McpHealth::Unknown`].
pub fn check_in_background(servers: &[McpServer]) -> std::sync::mpsc::Receiver<Answer> {
    let (tx, rx) = std::sync::mpsc::channel::<Answer>();
    // Claude runs the project's `.mcp.json` servers too, and its list reports on them.
    let checks: [(McpScope, &[McpScope]); 2] = [(McpScope::Claude, &[McpScope::Claude, McpScope::Project]), (McpScope::Codex, &[McpScope::Codex])];
    for (cli, covers) in checks {
        let wanted: Vec<(String, McpScope)> =
            servers.iter().flat_map(|s| s.presences.iter().filter(|p| covers.contains(&p.scope)).map(|p| (s.name.clone(), p.scope))).collect();
        if wanted.is_empty() {
            continue;
        }
        let tx = tx.clone();
        std::thread::spawn(move || {
            let said = check_reusing(cli);
            for (name, scope) in wanted {
                let health = match &said {
                    Ok(said) => said.iter().find(|(n, _)| *n == name).map_or(McpHealth::Unknown, |(_, h)| h.clone()),
                    Err(why) => McpHealth::Failed(why.clone()),
                };
                // Whoever asked may have stopped listening; then the answer is not needed.
                if tx.send((name, scope, health)).is_err() {
                    return;
                }
            }
        });
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = "Checking MCP server health…\n\n\
plugin:playwright:playwright: npx @playwright/mcp@latest - ✔ Connected\n\
posthog: https://mcp.posthog.com/mcp (HTTP) - \x1b[32m✔ Connected\x1b[0m\n\
broken: https://x.example/mcp (HTTP) - ✗ Failed to connect\n\
locked: https://y.example/mcp (HTTP) - ⚠ Needs authentication\n\
fresh: node s.js - ⏸ Pending approval\n";

    #[test]
    fn claude_lines_become_states() {
        let got = parse_claude_list(CLAUDE);
        assert_eq!(
            got,
            vec![
                ("plugin:playwright:playwright".to_string(), McpHealth::Connected),
                ("posthog".to_string(), McpHealth::Connected),
                ("broken".to_string(), McpHealth::Failed("Failed to connect".into())),
                ("locked".to_string(), McpHealth::NeedsLogin),
                ("fresh".to_string(), McpHealth::Failed("pending approval in claude".into())),
            ]
        );
    }

    #[test]
    fn claude_headers_and_blank_lines_are_skipped() {
        assert!(parse_claude_list("Checking MCP server health…\n\nNo MCP servers configured.\n").is_empty());
    }

    const CODEX: &str = r#"[
 {"name":"lunagraph","enabled":true,"disabled_reason":null,"transport":{"type":"streamable_http","url":"http://127.0.0.1:23563/mcp"},"auth_status":"unsupported"},
 {"name":"lunagraph-remote","enabled":true,"transport":{"type":"streamable_http"},"auth_status":"o_auth"},
 {"name":"off","enabled":false,"auth_status":"not_logged_in"},
 {"name":"locked","enabled":true,"auth_status":"not_logged_in"}
]"#;

    #[test]
    fn codex_json_says_off_or_login_and_nothing_more() {
        assert_eq!(
            parse_codex_json(CODEX),
            vec![
                ("lunagraph".to_string(), McpHealth::Unknown),
                ("lunagraph-remote".to_string(), McpHealth::Unknown),
                ("off".to_string(), McpHealth::Disabled),
                ("locked".to_string(), McpHealth::NeedsLogin),
            ]
        );
        assert!(parse_codex_json("not json").is_empty());
    }

    #[test]
    fn providers_without_a_list_command_answer_nothing() {
        assert_eq!(check(McpScope::Gemini), Ok(Vec::new()));
        assert_eq!(check(McpScope::Project), Ok(Vec::new()));
    }

    #[test]
    fn a_check_is_kept_for_a_minute_and_forgotten_on_demand() {
        let kept = || RECENT.lock().unwrap().iter().any(|k| k.scope == McpScope::Gemini);
        assert_eq!(check_reusing(McpScope::Gemini), Ok(Vec::new()));
        assert!(kept(), "a successful check is kept");
        assert_eq!(check_reusing(McpScope::Gemini), Ok(Vec::new()));
        forget_checks();
        assert!(!kept(), "forgotten");
    }

    #[test]
    fn a_slow_or_failing_command_is_an_error() {
        let slow = run("sleep", &["5"], Duration::from_millis(200));
        assert!(slow.as_ref().is_err_and(|e| e.contains("longer than")), "{slow:?}");
        let failing = run("sh", &["-c", "echo nope >&2; exit 3"], CHECK_TIMEOUT);
        assert!(failing.as_ref().is_err_and(|e| e.contains("nope")), "{failing:?}");
        assert!(run("jev-no-such-program", &[], CHECK_TIMEOUT).is_err());
    }
}
