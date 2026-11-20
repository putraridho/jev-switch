//! The scripted fake backend: a "CLI" that is a shell script printing canned lines, so the
//! runner, the turn engine, the delegator and the REPL are tested without any real CLI.
//!
//! Line grammar: `FIN:text` (finish), `FAIL:message`, `TOOL:summary`, `TEXT:text`; any other
//! line is ignored. What differs between tests is set on the [`ScriptedCli`] itself.

use std::sync::{Arc, Mutex};

use tokio::process::Command;

use crate::backend::{classify_common, Backend, ErrorKind, Event, Finish, McpAttachment, ProviderId, TurnRequest, Usage};

/// What the session id of a finished turn is.
#[derive(Clone)]
pub enum SessionId {
    /// No session id.
    Absent,
    /// The same id for every provider.
    Fixed(&'static str),
    /// `sess-<provider>`.
    PerProvider,
}

type OnStart = Box<dyn Fn(&TurnRequest) + Send + Sync>;
type OnFail = Box<dyn Fn() + Send + Sync>;

/// MCP attachments a fake backend was given, with the provider that got each.
pub type Attached = Arc<Mutex<Vec<(ProviderId, McpAttachment)>>>;

/// A backend whose CLI is `sh -c <script>` in the request's directory.
pub struct ScriptedCli {
    provider: ProviderId,
    script: String,
    session_id: SessionId,
    usage: Usage,
    prompt_env: bool,
    on_start: Option<OnStart>,
    on_fail: Option<OnFail>,
    attached: Option<Attached>,
}

impl ScriptedCli {
    /// A fake of `provider` running `script`. A finished turn has session id `s1` and
    /// 7 output tokens, as the runner's own tests expect.
    pub fn new(provider: ProviderId, script: impl Into<String>) -> Self {
        Self {
            provider,
            script: script.into(),
            session_id: SessionId::Fixed("s1"),
            usage: Usage { output_tokens: 7, ..Usage::default() },
            prompt_env: false,
            on_start: None,
            on_fail: None,
            attached: None,
        }
    }

    /// The session id a finished turn reports.
    pub fn session_id(mut self, id: SessionId) -> Self {
        self.session_id = id;
        self
    }

    /// The usage a finished turn reports.
    pub fn usage(mut self, usage: Usage) -> Self {
        self.usage = usage;
        self
    }

    /// Give the script the prompt and the session it was asked to resume, as `FAKE_PROMPT`
    /// and `FAKE_SESSION_ID`.
    pub fn with_prompt_env(mut self) -> Self {
        self.prompt_env = true;
        self
    }

    /// Call `f` with each request, as the command for it is built.
    pub fn on_start(mut self, f: impl Fn(&TurnRequest) + Send + Sync + 'static) -> Self {
        self.on_start = Some(Box::new(f));
        self
    }

    /// Call `f` each time the script prints a `FAIL:` line.
    pub fn on_fail(mut self, f: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_fail = Some(Box::new(f));
        self
    }

    /// Behave like a CLI that can take an MCP server (claude, deepseek, codex), and record
    /// each attachment in `attached`.
    pub fn recording_mcp(mut self, attached: Attached) -> Self {
        self.attached = Some(attached);
        self
    }
}

impl Backend for ScriptedCli {
    fn provider(&self) -> ProviderId {
        self.provider
    }

    fn command(&self, req: &TurnRequest) -> Command {
        if let Some(f) = &self.on_start {
            f(req);
        }
        let mut c = Command::new("sh");
        c.arg("-c").arg(&self.script).current_dir(&req.dir);
        if self.prompt_env {
            c.env("FAKE_PROMPT", &req.prompt).env("FAKE_SESSION_ID", req.session_id.as_deref().unwrap_or(""));
        }
        c
    }

    fn parse_line(&self, line: &str) -> Vec<Event> {
        let ev = match line.trim_end().split_once(':') {
            Some(("FIN", t)) => Event::Finished(Finish {
                text: t.into(),
                session_id: match &self.session_id {
                    SessionId::Absent => None,
                    SessionId::Fixed(id) => Some((*id).into()),
                    SessionId::PerProvider => Some(format!("sess-{}", self.provider)),
                },
                usage: self.usage,
            }),
            Some(("FAIL", m)) => {
                if let Some(f) = &self.on_fail {
                    f();
                }
                Event::Failed { message: m.into() }
            }
            Some(("TOOL", t)) => Event::Tool(t.into()),
            Some(("TEXT", t)) => Event::Text(t.into()),
            _ => return vec![],
        };
        vec![ev]
    }

    fn classify_error(&self, message: &str) -> ErrorKind {
        classify_common(message).unwrap_or(ErrorKind::Other)
    }

    fn supports_mcp(&self) -> bool {
        self.attached.is_some() && matches!(self.provider, ProviderId::Claude | ProviderId::DeepSeek | ProviderId::Codex)
    }

    fn attach_mcp(&mut self, mcp: &McpAttachment) {
        if let Some(attached) = &self.attached {
            attached.lock().unwrap().push((self.provider, mcp.clone()));
        }
    }
}
