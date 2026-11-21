//! A tool call's input as approval reads it: the fields the CLI sent, in the order the CLI's
//! JSON gives them, made domain values once at the adapter
//! (`approval::infrastructure::tool_input_wire`).
//!
//! Owns reading the fields a rule, a review or a summary needs. Must not parse or print JSON:
//! a value that is not text, a flag or a list (a number, a nested object) arrives already as
//! its compact JSON text, which is how the review shows it.

/// The input of one tool call.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolInput {
    shape: Shape,
    /// The whole input as the CLI sent it (compact JSON), for adapters that must send it back
    /// unchanged. The rules never read it.
    raw: String,
}

/// What the input is.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Shape {
    /// No input at all.
    #[default]
    Nothing,
    /// Named fields: the usual shape of a tool call.
    Fields(Vec<(String, InputValue)>),
    /// Anything else (a bare string, a number, a list): shown as its raw text.
    Other,
}

/// One field's value.
#[derive(Debug, Clone, PartialEq)]
pub enum InputValue {
    Text(String),
    Flag(bool),
    /// A list (MultiEdit's `edits`): each element as an input of its own, and the whole list
    /// as compact JSON text.
    List { text: String, items: Vec<ToolInput> },
    /// Anything else (a number, null, a nested object), as compact JSON text.
    Other(String),
}

impl InputValue {
    /// The value as the review prints it: text as it is, anything else as compact JSON.
    pub fn text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Flag(b) => b.to_string(),
            Self::List { text, .. } | Self::Other(text) => text.clone(),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Flag(b) => Some(*b),
            _ => None,
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    /// The elements of a list; none for anything else.
    pub fn items(&self) -> &[ToolInput] {
        match self {
            Self::List { items, .. } => items,
            _ => &[],
        }
    }
}

impl ToolInput {
    pub fn new(shape: Shape, raw: String) -> Self {
        Self { shape, raw }
    }

    /// An input of named fields, for callers (and tests) that build one directly. Its raw
    /// form is left empty: only an adapter knows the CLI's exact text.
    pub fn from_fields(fields: Vec<(String, InputValue)>) -> Self {
        Self { shape: Shape::Fields(fields), raw: String::new() }
    }

    pub fn shape(&self) -> &Shape {
        &self.shape
    }

    /// The input exactly as the CLI sent it.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// The named fields, in order; none when the input is not an object.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &InputValue)> {
        let list: &[(String, InputValue)] = match &self.shape {
            Shape::Fields(f) => f,
            _ => &[],
        };
        list.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn get(&self, key: &str) -> Option<&InputValue> {
        self.fields().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    /// A text field's value.
    pub fn str_field(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(InputValue::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_read_by_name_and_only_text_is_a_str() {
        let input = ToolInput::from_fields(vec![("command".into(), InputValue::Text("ls".into())), ("timeout".into(), InputValue::Other("5".into()))]);
        assert_eq!(input.str_field("command"), Some("ls"));
        assert_eq!(input.str_field("timeout"), None);
        assert_eq!(input.get("timeout").map(InputValue::text).as_deref(), Some("5"));
        assert_eq!(input.get("missing"), None);
        assert_eq!(ToolInput::default().fields().count(), 0);
    }
}
