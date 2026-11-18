//! `/export`: the session as a document to share. Markdown or a self-contained HTML page,
//! chosen by the file's extension, and always redacted (see [`crate::redact`]): a transcript
//! that leaves the machine must not carry a pasted key.
//!
//! The Markdown itself is [`SessionLog::to_markdown`]; this module adds the redaction and the
//! HTML page, and decides the format from a path.

use std::path::Path;

use crate::redact::redact;
use crate::sessionlog::{SessionLog, TurnRecord};
use crate::turn::TurnStatus;

/// What a file extension asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Html,
}

impl Format {
    /// `.html` and `.htm` are HTML, everything else (and no extension) Markdown.
    pub fn of_path(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("html" | "htm") => Format::Html,
            _ => Format::Markdown,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Markdown => "md",
            Format::Html => "html",
        }
    }
}

/// `text` safe to put inside HTML.
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn status_note(status: TurnStatus) -> &'static str {
    match status {
        TurnStatus::Answered => "",
        TurnStatus::Cancelled => "stopped",
        TurnStatus::Failed => "the turn failed",
        TurnStatus::NoRoute => "no model could take the turn",
    }
}

fn turn_html(n: usize, t: &TurnRecord) -> String {
    let who = if t.model.is_empty() { "jev-code" } else { t.model.as_str() };
    let body = match status_note(t.status) {
        "" => format!("<pre>{}</pre>", escape_html(&redact(t.answer.trim_end()))),
        note => format!("<p class=\"note\">{note}</p>"),
    };
    format!(
        "<section><h2>{n}. You</h2><blockquote>{}</blockquote><h3>{}</h3>{body}</section>\n",
        escape_html(&redact(&t.prompt)).replace('\n', "<br>"),
        escape_html(who)
    )
}

const STYLE: &str = "body{font:15px/1.5 system-ui,sans-serif;max-width:50rem;margin:2rem auto;padding:0 1rem;color:#1b1f23}\
blockquote{margin:.5rem 0;padding:.4rem .9rem;border-left:4px solid #8aa;background:#f3f6f8}\
pre{white-space:pre-wrap;background:#f6f8fa;padding:.8rem;border-radius:6px}\
h3{color:#555;font-size:.95rem;margin-bottom:.2rem}.note{color:#a60;font-style:italic}\
@media(prefers-color-scheme:dark){body{background:#14171a;color:#d8dde1}blockquote{background:#1d2328}pre{background:#1d2328}}";

/// The session as one self-contained HTML page (no scripts, no external files).
pub fn to_html(log: &SessionLog, project: &str, started: &str) -> String {
    let mut out = format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>jev-code session</title><style>{STYLE}</style></head><body>\n\
         <h1>jev-code session</h1><ul><li>Project: <code>{}</code></li><li>Exported: {}</li><li>Turns: {}</li></ul>\n",
        escape_html(project),
        escape_html(started),
        log.turns().len()
    );
    for (i, t) in log.turns().iter().enumerate() {
        out.push_str(&turn_html(i + 1, t));
    }
    out.push_str("</body></html>\n");
    out
}

/// The document for `format`, redacted.
pub fn render(log: &SessionLog, format: Format, project: &str, started: &str) -> String {
    match format {
        Format::Markdown => redact(&log.to_markdown(project, started)),
        Format::Html => to_html(log, project, started),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_with(prompt: &str, answer: &str, status: TurnStatus) -> SessionLog {
        let mut log = SessionLog::default();
        log.push(TurnRecord { prompt: prompt.into(), answer: answer.into(), status, model: "claude · s".into(), route: None, usage: None, elapsed_ms: 1 });
        log
    }

    #[test]
    fn the_extension_picks_the_format() {
        assert_eq!(Format::of_path(Path::new("a/b.HTML")), Format::Html);
        assert_eq!(Format::of_path(Path::new("b.htm")), Format::Html);
        assert_eq!(Format::of_path(Path::new("b.md")), Format::Markdown);
        assert_eq!(Format::of_path(Path::new("notes")), Format::Markdown);
    }

    #[test]
    fn html_escapes_everything_and_has_no_script() {
        let log = log_with("<script>alert(1)</script> & \"q\"", "a < b > c", TurnStatus::Answered);
        let html = to_html(&log, "/p<q>", "2026-01-01");
        assert!(!html.contains("<script>alert") && html.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;q&quot;"), "{html}");
        assert!(html.contains("a &lt; b &gt; c") && html.contains("/p&lt;q&gt;"));
        assert!(html.starts_with("<!doctype html>") && html.trim_end().ends_with("</html>"));
    }

    #[test]
    fn both_formats_are_redacted() {
        let log = log_with("my key is sk-ant-api03-abcdefghijklmnopqrstuvwxyz", "set API_KEY=hunter2hunter2 in .env", TurnStatus::Answered);
        for format in [Format::Markdown, Format::Html] {
            let doc = render(&log, format, "/p", "now");
            assert!(doc.contains("[REDACTED]"), "{doc}");
            assert!(!doc.contains("abcdefghijklmnop") && !doc.contains("hunter2"), "{doc}");
        }
    }

    #[test]
    fn a_stopped_turn_says_so_in_html() {
        let html = to_html(&log_with("go", "", TurnStatus::Cancelled), "/p", "now");
        assert!(html.contains("<p class=\"note\">stopped</p>"), "{html}");
    }
}
