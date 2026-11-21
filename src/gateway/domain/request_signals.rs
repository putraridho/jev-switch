//! Pure helpers over a request body and prompt text: which settings are worth logging,
//! the project directory Claude Code reports, and prompt similarity.

use serde_json::{json, Map, Value};

const CWD_KEY: &str = "Primary working directory: ";

/// Request settings worth analyzing: every top-level field except the content.
/// Large values are replaced by their size.
pub fn request_params(req: &Value) -> Value {
    const SKIP: &[&str] = &["messages", "system", "tools", "model", "stream", "metadata"];
    let mut out = Map::new();
    for (k, v) in req.as_object().into_iter().flatten() {
        if SKIP.contains(&k.as_str()) {
            continue;
        }
        let n = v.to_string().len();
        out.insert(k.clone(), if n > 1024 { json!(format!("<{n} bytes>")) } else { v.clone() });
    }
    Value::Object(out)
}

/// The project directory Claude Code reports, in the system prompt or in a
/// text block of the conversation (after a compaction it is no longer in the first
/// messages). Tool results are not searched: files may quote the key.
pub fn find_cwd(req: &Value) -> Option<String> {
    let in_messages = || req.get("messages")?.as_array()?.iter().find_map(|m| cwd_in(m.get("content")?));
    req.get("system").and_then(cwd_in).or_else(in_messages)
}

fn cwd_in(content: &Value) -> Option<String> {
    let from_text = |t: &str| {
        let line = t[t.find(CWD_KEY)? + CWD_KEY.len()..].lines().next()?.trim();
        (!line.is_empty()).then(|| line.to_string())
    };
    match content {
        Value::String(s) => from_text(s),
        Value::Array(bs) => bs.iter().find_map(|b| b.get("text").and_then(Value::as_str).and_then(from_text)),
        _ => None,
    }
}

pub fn project_of(cwd: &str) -> &str {
    cwd.trim_end_matches('/').rsplit('/').next().unwrap_or(cwd)
}

/// Jaccard overlap of the word sets: near 1.0 means the same question asked again.
pub fn similarity(a: &str, b: &str) -> f64 {
    let words = |s: &str| -> std::collections::HashSet<String> {
        s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
    };
    let (a, b) = (words(a), words(b));
    let union = a.union(&b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(&b).count() as f64 / union as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_cwd_and_project() {
        let req = json!({"messages": [{"role": "user", "content": [{"type": "text",
            "text": "x\n - Primary working directory: /home/u/Playground/app\n - Is a git repository: true"}]}]});
        let cwd = find_cwd(&req).unwrap();
        assert_eq!(cwd, "/home/u/Playground/app");
        assert_eq!(project_of(&cwd), "app");
        assert_eq!(find_cwd(&json!({"messages": []})), None);
    }

    #[test]
    fn cwd_found_in_system_prompt_or_late_message_but_not_in_tool_results() {
        let line = "Primary working directory: /home/u/app";
        let in_system = json!({"system": [{"type": "text", "text": format!("# Environment\n - {line}")}], "messages": []});
        assert_eq!(find_cwd(&in_system).as_deref(), Some("/home/u/app"));
        let msg = |c: Value| json!({"role": "user", "content": c});
        let filler = || msg(json!("hi"));
        let late = json!({"messages": [filler(), filler(), filler(), filler(), msg(json!([{"type": "text", "text": line}]))]});
        assert_eq!(find_cwd(&late).as_deref(), Some("/home/u/app"));
        let quoted = json!({"messages": [msg(json!([{"type": "tool_result", "content": line}]))]});
        assert_eq!(find_cwd(&quoted), None);
    }

    #[test]
    fn request_params_drop_content_and_cap_big_values() {
        let req = json!({"model": "m", "messages": [1], "system": "s", "tools": [], "stream": true,
            "max_tokens": 10, "big": "x".repeat(2000)});
        let p = request_params(&req);
        assert_eq!(p["max_tokens"], 10);
        assert!(p.get("messages").is_none() && p.get("model").is_none());
        assert!(p["big"].as_str().unwrap().starts_with("<20"));
    }

    #[test]
    fn rephrase_scores_high_new_task_low() {
        assert!(similarity("fix the upload hang", "fix upload hang please") > 0.5);
        assert!(similarity("fix the upload hang", "add a unit test") < 0.2);
        assert_eq!(similarity("", ""), 0.0);
    }
}
