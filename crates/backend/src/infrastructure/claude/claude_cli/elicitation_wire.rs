//! Claude's SDK `elicitation` control request and response. Keep wire schemas here.

use serde_json::{json, Value};
use crate::{ElicitationRequest, ElicitationKind, ElicitationField, ElicitationAnswer, ElicitationValue, FieldKind};

pub(super) fn request(ev: &Value) -> Option<ElicitationRequest> {
    let r = &ev["request"];
    if r["subtype"] != "elicitation" { return None; }
    let kind = match r.get("mode").map_or("form", |m| m.as_str().unwrap_or("unsupported")) {
        "form" => form(&r["requested_schema"], &r["property_order"]).map(ElicitationKind::Form),
        "url" => r["url"].as_str().filter(|u| u.starts_with("https://") || u.starts_with("http://"))
            .map(|u| ElicitationKind::Url(u.into())).ok_or_else(|| "The server supplied an unsupported URL".into()),
        _ => Err("This elicitation mode is not supported".into()),
    }.unwrap_or_else(ElicitationKind::Unsupported);
    Some(ElicitationRequest {
        id: ev["request_id"].as_str()?.into(),
        server: r["mcp_server_name"].as_str().unwrap_or("MCP server").into(),
        message: r["message"].as_str().unwrap_or("The server needs your input").into(),
        kind,
    })
}

fn form(schema: &Value, order: &Value) -> Result<Vec<ElicitationField>, String> {
    if schema["type"] != "object" { return Err("The server requested an unsupported form schema".into()); }
    let empty = serde_json::Map::new();
    let properties = match schema.get("properties") {
        Some(p) => p.as_object().ok_or("The form properties are invalid")?,
        None => &empty,
    };
    let required: Vec<&str> = match schema.get("required") {
        None => Vec::new(),
        Some(r) => r.as_array().ok_or("The required fields are invalid")?.iter()
            .map(|v| v.as_str().ok_or("A required field name is invalid")).collect::<Result<_, _>>()?,
    };
    if required.iter().any(|name| !properties.contains_key(*name)) { return Err("A required field has no schema".into()); }
    let mut names: Vec<&str> = order.as_array().map(|a| a.iter().filter_map(Value::as_str).filter(|n| properties.contains_key(*n)).collect()).unwrap_or_default();
    for name in properties.keys() {
        if !names.contains(&name.as_str()) { names.push(name); }
    }
    let mut fields = Vec::new();
    for name in names {
        if fields.iter().any(|f: &ElicitationField| f.name == name) { continue; }
        let p = &properties[name];
        if ["oneOf", "anyOf", "allOf", "$ref"].iter().any(|key| p.get(key).is_some()) {
            return Err(format!("The field {name} uses an unsupported schema"));
        }
        let kind = match p["type"].as_str() {
            Some("string") if p.get("enum").is_some() => {
                let choices = p["enum"].as_array().ok_or("Invalid choices")?.iter().map(|v| v.as_str().map(String::from).ok_or("Invalid choice")).collect::<Result<Vec<_>, _>>()?;
                if choices.is_empty() { return Err("The server supplied no choices".into()); }
                FieldKind::Choice(choices)
            }
            Some("string") => FieldKind::Text,
            Some("boolean") => FieldKind::Boolean,
            Some("integer") => FieldKind::Integer,
            Some("number") => FieldKind::Number,
            _ => return Err(format!("The field {name} uses an unsupported type")),
        };
        let default = p.get("default").map(|v| match v { Value::String(s) => s.clone(), Value::Bool(_) | Value::Number(_) => v.to_string(), _ => String::new() }).unwrap_or_default();
        fields.push(ElicitationField {
            name: name.into(), title: p["title"].as_str().unwrap_or(name).into(),
            description: p["description"].as_str().unwrap_or("").into(),
            required: required.contains(&name), kind, default,
            min: p["minimum"].as_f64(), max: p["maximum"].as_f64(),
            min_length: p["minLength"].as_u64().and_then(|n| usize::try_from(n).ok()),
            max_length: p["maxLength"].as_u64().and_then(|n| usize::try_from(n).ok()),
        });
    }
    Ok(fields)
}

pub(super) fn response(req: &ElicitationRequest, answer: &ElicitationAnswer) -> String {
    let answer = match answer {
        ElicitationAnswer::Accept(fields) => {
            let content: serde_json::Map<String, Value> = fields.iter().map(|(name, value)| {
                let value = match value {
                    ElicitationValue::Text(s) => json!(s), ElicitationValue::Boolean(b) => json!(b),
                    ElicitationValue::Integer(n) => json!(n), ElicitationValue::Number(n) => json!(n),
                };
                (name.clone(), value)
            }).collect();
            match req.kind { ElicitationKind::Url(_) => json!({"action":"accept"}), _ => json!({"action":"accept","content":content}) }
        }
        ElicitationAnswer::Decline => json!({"action":"decline"}),
        ElicitationAnswer::Cancel => json!({"action":"cancel"}),
    };
    json!({"type":"control_response","response":{"subtype":"success","request_id":req.id,"response":answer}}).to_string()
}
