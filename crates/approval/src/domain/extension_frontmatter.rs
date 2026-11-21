//! The small header markdown extension files may start with:
//!
//! ```text
//! ---
//! description: Fix the failing tests
//! hint: [test name]
//! mode: edit
//! ---
//! the body
//! ```
//!
//! Only these keys are read; others are ignored so files written for other tools still load.

use crate::domain::mode::Mode;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    pub description: String,
    /// What the argument is, shown after the name in listings.
    pub hint: String,
    /// The permission mode the entry runs under (then put back).
    pub mode: Option<Mode>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub meta: Meta,
    pub body: String,
}

fn unquote(value: &str) -> &str {
    let v = value.trim();
    v.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(v)
}

/// Split `text` into its header and body. A file with no header is all body. A header that is
/// opened and never closed, or names a mode that does not exist, is an error with a reason.
pub fn parse(text: &str) -> Result<Document, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else {
        return Ok(Document { meta: Meta::default(), body: text.trim().to_string() });
    };
    let mut meta = Meta::default();
    let mut consumed = 0;
    let mut closed = false;
    for line in rest.split_inclusive('\n') {
        consumed += line.len();
        let line = line.trim_end_matches(['\n', '\r']);
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = unquote(value);
        match key.trim() {
            "description" => meta.description = value.to_string(),
            "hint" | "argument-hint" => meta.hint = value.to_string(),
            "mode" => meta.mode = Some(value.parse().map_err(|e: crate::domain::mode::UnknownMode| e.to_string())?),
            _ => {}
        }
    }
    if !closed {
        return Err("the header opened with --- is never closed".to_string());
    }
    Ok(Document { meta, body: rest[consumed..].trim().to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_known_keys_and_the_body() {
        let d = parse("---\ndescription: \"Fix tests\"\nargument-hint: [name]\nmode: plan\nother: x\n---\n\nDo it.\n").unwrap();
        assert_eq!(d.meta, Meta { description: "Fix tests".into(), hint: "[name]".into(), mode: Some(Mode::Plan) });
        assert_eq!(d.body, "Do it.");
    }

    #[test]
    fn no_header_is_all_body() {
        let d = parse("just a prompt\n").unwrap();
        assert_eq!((d.meta, d.body.as_str()), (Meta::default(), "just a prompt"));
        assert_eq!(parse("--- not a header\nx").unwrap().body, "--- not a header\nx");
    }

    #[test]
    fn bad_headers_say_why() {
        assert!(parse("---\ndescription: x\nno end").unwrap_err().contains("never closed"));
        assert!(parse("---\nmode: yolo\n---\nx").unwrap_err().contains("ask, edit, plan or auto"));
    }
}
