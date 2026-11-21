//! The request body as Anthropic receives it: the model swapped in place.

use axum::body::Bytes;
use serde_json::Value;

use crate::domain::model_choice::ALIAS;

/// Replace the model value, leaving every other byte of the body as sent.
pub fn set_model(body: &Bytes, model: &str) -> Bytes {
    let needle = format!("\"model\":\"{ALIAS}\"");
    let text = String::from_utf8_lossy(body);
    if let Some(i) = text.find(&needle) {
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..i]);
        out.push_str(&format!("\"model\":\"{model}\""));
        out.push_str(&text[i + needle.len()..]);
        return Bytes::from(out);
    }
    // Unusual spacing: fall back to a JSON round-trip (same content, new whitespace).
    match serde_json::from_slice::<Value>(body) {
        Ok(mut v) => {
            v["model"] = Value::String(model.into());
            serde_json::to_vec(&v).map(Bytes::from).unwrap_or_else(|_| body.clone())
        }
        Err(_) => body.clone(),
    }
}
