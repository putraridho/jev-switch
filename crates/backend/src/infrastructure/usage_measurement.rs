use serde_json::Value;

use crate::Measurement;

pub(super) fn measurement_of(usage: &Value) -> Measurement {
    if ["input_tokens", "output_tokens"].iter().all(|key| usage[*key].as_u64().is_some()) {
        Measurement::Reported
    } else {
        Measurement::Unavailable
    }
}

#[cfg(test)]
pub(super) fn test_cases() -> Vec<(Option<Value>, Measurement)> {
    use serde_json::json;

    vec![
        (None, Measurement::Unavailable),
        (Some(Value::Null), Measurement::Unavailable),
        (Some(json!({})), Measurement::Unavailable),
        (Some(json!({"input_tokens":0})), Measurement::Unavailable),
        (Some(json!({"output_tokens":0})), Measurement::Unavailable),
        (Some(json!({"input_tokens":null,"output_tokens":0})), Measurement::Unavailable),
        (Some(json!({"input_tokens":0,"output_tokens":null})), Measurement::Unavailable),
        (Some(json!({"input_tokens":"0","output_tokens":0})), Measurement::Unavailable),
        (Some(json!({"input_tokens":0,"output_tokens":"0"})), Measurement::Unavailable),
        (Some(json!({"input_tokens":-1,"output_tokens":0})), Measurement::Unavailable),
        (Some(json!({"input_tokens":0,"output_tokens":-1})), Measurement::Unavailable),
        (Some(json!({"input_tokens":0.0,"output_tokens":0})), Measurement::Unavailable),
        (Some(json!({"input_tokens":0,"output_tokens":0.0})), Measurement::Unavailable),
        (Some(json!({"input_tokens":0,"output_tokens":0})), Measurement::Reported),
    ]
}
