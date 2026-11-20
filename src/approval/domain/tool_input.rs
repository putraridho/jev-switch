//! Reading the fields of a tool call's JSON input.

use serde_json::Value;

pub(super) fn str_field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

/// A field's text: a string as it is, anything else as compact JSON.
pub(super) fn value_text(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), String::from)
}

pub(super) fn fields(input: &Value) -> impl Iterator<Item = (&String, &Value)> {
    input.as_object().into_iter().flat_map(|m| m.iter())
}
