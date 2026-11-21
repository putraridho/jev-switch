//! What goes to stdin: prompts, images and mid-turn messages.

use super::*;
use crate::backend::infrastructure::claude::claude_cli::stream_input::MediaType;

#[test]
fn image_input_and_base64() {
    let backend = backend().in_session();
    assert_eq!(backend.stdin_payload(&req(None)).unwrap(), "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n");
    let dir = std::env::temp_dir().join(format!("jev-claude-image-{}", std::process::id()));
    // A JPEG saved as .png goes as what it really is.
    std::fs::write(&dir, [0xff, 0xd8, 0xff, 0xe0]).unwrap();
    let path = dir.with_extension("png");
    std::fs::rename(&dir, &path).unwrap();
    let mut request = req(None);
    request.images.push(path.clone());
    let payload = backend.stdin_payload(&request).unwrap();
    assert!(payload.contains("\"type\":\"image\"") && payload.contains("\"media_type\":\"image/jpeg\""), "{payload}");
    assert!(payload.contains("\"data\":\"/9j/4A==\""), "{payload}");
    // Text with an image name is left out with a note, never sent as an image.
    let fake = std::env::temp_dir().join(format!("jev-claude-fake-{}.png", std::process::id()));
    std::fs::write(&fake, b"not an image").unwrap();
    let mut faked = req(None);
    faked.images.push(fake.clone());
    let note = backend.stdin_payload(&faked).unwrap();
    assert!(!note.contains("\"type\":\"image\"") && note.contains("not a PNG, JPEG, GIF or WebP"), "{note}");
    std::fs::remove_file(fake).unwrap();
    assert_eq!(MediaType::sniff(b"\x89PNG\r\n\x1a\nrest"), Some(MediaType::Png));
    assert_eq!(MediaType::sniff(b"GIF89a"), Some(MediaType::Gif));
    assert_eq!(MediaType::sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(MediaType::Webp));
    assert_eq!(MediaType::sniff(b"RIFF"), None);
    // DeepSeek's API takes no images: it gets their paths as text.
    let deepseek = ClaudeBackend::with_profile(ProviderId::DeepSeek, vec![]).in_session();
    let text = deepseek.stdin_payload(&request).unwrap();
    assert!(!text.contains("\"type\":\"image\"") && text.contains(&*path.to_string_lossy()), "{text}");
    let plain = ClaudeBackend::with_profile(ProviderId::DeepSeek, vec![]);
    let args: Vec<String> = plain.command(&request).as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert!(!args.contains(&"stream-json".to_string()) || args.windows(2).all(|w| w != ["--input-format", "stream-json"]), "{args:?}");
    assert!(plain.stdin_payload(&request).unwrap().contains("Attached images"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_session_takes_a_message_mid_turn_and_reports_when_the_model_has_it() {
    let line = session().steer_line("also run clippy").unwrap();
    assert_eq!(json_of(&line), json!({"type": "user", "message": {"role": "user", "content": "also run clippy"}}));
    assert!(session().steerable());
    // Without the host protocol stdin is closed after the prompt: nothing can be added.
    assert!(backend().steer_line("x").is_none() && !backend().steerable());
    let replay = |content: &str| format!(r#"{{"type":"user","message":{{"role":"user","content":{content}}},"isReplay":true,"session_id":"s"}}"#);
    let steered = |l: String| backend().parse_line(&l).into_iter().map(|e| match e {
        Event::Steered(t) => t,
        other => panic!("{other:?}"),
    }).collect::<Vec<_>>();
    assert_eq!(steered(replay(r#""also run clippy""#)), ["also run clippy"]);
    assert_eq!(steered(replay(r#"[{"type":"text","text":"a"},{"type":"text","text":"b"}]"#)), ["a\nb"]);
    // A tool result is still a tool result.
    let result = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}}"#;
    assert!(matches!(backend().parse_line(result).as_slice(), [Event::ToolResult { ok: true, .. }]));
}

#[test]
fn the_prompt_travels_as_a_stream_json_user_message() {
    let mut request = req(None);
    request.prompt = "Use the Bash tool to run exactly: touch /tmp/spike/outside.txt   Then reply with just the word done.".into();
    let payload = session().stdin_payload(&request).unwrap();
    assert!(payload.ends_with('\n') && payload.matches('\n').count() == 1);
    assert_eq!(json_of(&payload), json_of(USER_LINE));
    // A multi-line prompt stays on one line.
    request.prompt = "a\nb \"c\"".into();
    let payload = session().stdin_payload(&request).unwrap();
    assert_eq!(payload.matches('\n').count(), 1);
    assert_eq!(json_of(&payload)["message"]["content"], "a\nb \"c\"");
    // Headless: the bare prompt, as before.
    assert_eq!(backend().stdin_payload(&request).as_deref(), Some("a\nb \"c\""));
}
