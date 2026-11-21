//! The anti-corruption layer for tool-call inputs: a CLI sends them as JSON, approval reads
//! them as [`ToolInput`]. The one place that JSON becomes domain values, and back.

use serde_json::Value;

use crate::domain::tool_input::{InputValue, Shape, ToolInput};

/// `input` as the CLI sent it, made a [`ToolInput`].
pub fn tool_input_from_json(input: &Value) -> ToolInput {
    let shape = match input {
        Value::Null => Shape::Nothing,
        Value::Object(map) => Shape::Fields(map.iter().map(|(k, v)| (k.clone(), value(v))).collect()),
        _ => Shape::Other,
    };
    ToolInput::new(shape, input.to_string())
}

/// The input back as the JSON the CLI sent (for a reply that must echo it unchanged).
pub fn tool_input_to_json(input: &ToolInput) -> Value {
    serde_json::from_str(input.raw()).unwrap_or(Value::Null)
}

fn value(v: &Value) -> InputValue {
    match v {
        Value::String(s) => InputValue::Text(s.clone()),
        Value::Bool(b) => InputValue::Flag(*b),
        Value::Array(items) => InputValue::List { text: v.to_string(), items: items.iter().map(tool_input_from_json).collect() },
        other => InputValue::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_becomes_domain_values_and_comes_back_unchanged() {
        let raw = json!({"command": "ls", "timeout": 5, "flag": true, "edits": [{"old_string": "a"}, "x"], "nested": {"k": 1}, "nothing": null});
        let input = tool_input_from_json(&raw);
        assert_eq!(input.str_field("command"), Some("ls"));
        assert_eq!(input.get("timeout"), Some(&InputValue::Other("5".into())));
        assert_eq!(input.get("flag").and_then(InputValue::as_bool), Some(true));
        assert_eq!(input.get("nested").map(InputValue::text).as_deref(), Some(r#"{"k":1}"#));
        assert_eq!(input.get("nothing").map(InputValue::text).as_deref(), Some("null"));
        let edits = input.get("edits").unwrap();
        assert_eq!(edits.items().len(), 2);
        assert_eq!(edits.items()[0].str_field("old_string"), Some("a"));
        assert_eq!(edits.items()[1].shape(), &Shape::Other);
        assert_eq!(edits.text(), r#"[{"old_string":"a"},"x"]"#);
        assert_eq!(tool_input_to_json(&input), raw);
        assert_eq!(tool_input_from_json(&Value::Null).shape(), &Shape::Nothing);
        assert_eq!(tool_input_from_json(&json!("bare")).shape(), &Shape::Other);
    }
}
