//! The editor bridge protocol: what a VS Code, JetBrains or Neovim plugin says to a running
//! `jev-code`, and what it answers, as one JSON object per line (so any editor can speak it
//! over a pipe or socket with a few lines of glue).
//!
//! This module is the contract only: the typed messages, their wire form and their parsing.
//! The transport that carries them is not part of it.

use serde::{Deserialize, Serialize};

/// Bumped when a message changes in a way an old plugin cannot ignore.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// One editor diagnostic (a compiler or linter finding) at a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub line: u32,
    pub severity: Severity,
    pub message: String,
}

/// Editor to jev-code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// First message: who is speaking and which protocol it knows.
    Hello { client: String, protocol: u32 },
    /// The file now in focus, and its cursor line.
    Focus { path: String, line: u32 },
    /// Text the person selected, to be sent with the next message.
    Selection { path: String, start_line: u32, end_line: u32, text: String },
    /// The editor's current findings for one file.
    Diagnostics { path: String, items: Vec<Diagnostic> },
    /// A message to send as if it was typed.
    Prompt { text: String },
    /// Stop the running turn.
    Interrupt,
}

/// jev-code to the editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Ready { version: String, protocol: u32 },
    Ack,
    /// A file the agent changed, so the editor can reload or show it.
    FileChanged { path: String },
    /// An approval the agent is waiting for.
    ApprovalNeeded { tool: String, summary: String },
    TurnFinished { ok: bool },
    Error { message: String },
}

/// Why a line is not a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadLine(pub String);

impl std::fmt::Display for BadLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not a bridge request: {}", self.0)
    }
}

pub fn parse_request(line: &str) -> Result<Request, BadLine> {
    serde_json::from_str(line.trim()).map_err(|e| BadLine(e.to_string()))
}

/// The wire form of a response: one line, no trailing newline.
pub fn encode(response: &Response) -> String {
    serde_json::to_string(response).unwrap_or_else(|_| r#"{"type":"error","message":"internal"}"#.to_string())
}

/// The answer to a `hello`: ready when the protocols match, an error saying so when they do not.
pub fn greet(client_protocol: u32, version: &str) -> Response {
    if client_protocol == PROTOCOL_VERSION {
        Response::Ready { version: version.to_string(), protocol: PROTOCOL_VERSION }
    } else {
        Response::Error { message: format!("the plugin speaks protocol {client_protocol}, jev-code speaks {PROTOCOL_VERSION}; update the older one") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_parse_from_the_documented_wire_form() {
        assert_eq!(parse_request(r#"{"type":"hello","client":"vscode","protocol":1}"#), Ok(Request::Hello { client: "vscode".into(), protocol: 1 }));
        assert_eq!(parse_request(r#" {"type":"interrupt"} "#), Ok(Request::Interrupt));
        let d = parse_request(r#"{"type":"diagnostics","path":"a.rs","items":[{"line":3,"severity":"warning","message":"unused"}]}"#).unwrap();
        assert_eq!(d, Request::Diagnostics { path: "a.rs".into(), items: vec![Diagnostic { line: 3, severity: Severity::Warning, message: "unused".into() }] });
    }

    #[test]
    fn bad_lines_are_errors_not_panics() {
        for bad in ["", "nope", r#"{"type":"unknown"}"#, r#"{"type":"focus","path":"a"}"#] {
            assert!(parse_request(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_request_survives_a_round_trip() {
        let all = [
            Request::Hello { client: "nvim".into(), protocol: 1 },
            Request::Focus { path: "a".into(), line: 1 },
            Request::Selection { path: "a".into(), start_line: 1, end_line: 2, text: "x\ny".into() },
            Request::Diagnostics { path: "a".into(), items: vec![] },
            Request::Prompt { text: "hi".into() },
            Request::Interrupt,
        ];
        for r in all {
            assert_eq!(parse_request(&serde_json::to_string(&r).unwrap()), Ok(r));
        }
    }

    #[test]
    fn responses_encode_as_single_tagged_lines() {
        assert_eq!(encode(&Response::Ack), r#"{"type":"ack"}"#);
        assert_eq!(encode(&Response::TurnFinished { ok: true }), r#"{"type":"turn_finished","ok":true}"#);
        assert!(!encode(&Response::Error { message: "a\nb".into() }).contains('\n'));
    }

    #[test]
    fn a_protocol_mismatch_is_said_plainly() {
        assert_eq!(greet(1, "1.0.0"), Response::Ready { version: "1.0.0".into(), protocol: 1 });
        assert!(matches!(greet(9, "1.0.0"), Response::Error { message } if message.contains("protocol 9")));
    }
}
