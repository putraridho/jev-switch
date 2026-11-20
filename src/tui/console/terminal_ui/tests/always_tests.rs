use super::*;

/// A request for the recorded `touch /tmp/spike/outside.txt`, which Claude Code asked about
/// because the path is outside the allowed folders.
fn blocked_touch(command: &str) -> PermissionRequest {
    PermissionRequest { blocked_path: Some("/tmp/spike/outside.txt".into()), ..request("Bash", command) }
}

fn console_in(project: &std::path::Path, key: AnswerKey) -> (Console, Buf) {
    let err = Buf::default();
    let cfg = ConsoleConfig { out_style: Style::off(), err_style: Style::off(), width: 100, live: false, project: project.to_path_buf(), stream_answers: true };
    (Console::new(Box::new(Buf::default()), Box::new(err.clone()), cfg).with_key_reader(presses(key).0), err)
}

#[test]
fn always_for_a_path_blocked_call_saves_the_folder_and_says_exactly_what_was_saved() {
    let dir = tmp_project("blocked");
    let (mut c, err) = console_in(&dir, AnswerKey::Always);
    let d = ask(&mut c, &blocked_touch("touch /tmp/spike/outside.txt"));
    assert_eq!(d, Some(Decision::AllowAlways));
    let e = err.text();
    assert!(e.contains("[a]lways in this project"), "{e}");
    assert!(e.contains("[a] also lets Claude Code use everything in the folder /tmp/spike"), "{e}");
    assert!(e.trim_end().ends_with("✓ allowed, and always in this project: Bash(touch:*) and folder /tmp/spike"), "{e}");
    let saved = crate::approval::application::claude_settings_file::load(&crate::approval::domain::settings_document::local_settings_path(&dir)).unwrap();
    assert_eq!(saved.list(crate::approval::domain::settings_document::List::Directories), ["/tmp/spike"]);
    assert_eq!(saved.list(crate::approval::domain::settings_document::List::Allow), ["Bash(touch:*)"], "rule and folder in one write");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_folder_alone_is_offered_when_the_command_has_no_safe_rule() {
    let dir = tmp_project("blocked-alone");
    let (c, err) = console_in(&dir, AnswerKey::Always);
    let (keys, offered) = presses(AnswerKey::Always);
    let mut c = c.with_key_reader(keys);
    let d = ask(&mut c, &blocked_touch("bash -c 'touch /tmp/spike/outside.txt'"));
    assert_eq!(*offered.lock().unwrap(), vec![true]);
    assert_eq!(d, Some(Decision::AllowAlways));
    assert!(err.text().trim_end().ends_with("✓ allowed, and always in this project: folder /tmp/spike"), "{}", err.text());
    let saved = crate::approval::application::claude_settings_file::load(&crate::approval::domain::settings_document::local_settings_path(&dir)).unwrap();
    assert_eq!(saved.list(crate::approval::domain::settings_document::List::Directories), ["/tmp/spike"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_plain_yes_for_a_path_blocked_call_saves_nothing_and_a_broad_folder_is_not_offered() {
    let dir = tmp_project("blocked-yes");
    let (mut c, _) = console_in(&dir, AnswerKey::Yes);
    assert_eq!(ask(&mut c, &blocked_touch("touch /tmp/spike/outside.txt")), Some(Decision::Allow));
    assert!(!crate::approval::domain::settings_document::local_settings_path(&dir).exists());
    let (c, err) = console_in(&dir, AnswerKey::Yes);
    let (keys, offered) = presses(AnswerKey::Yes);
    let mut c = c.with_key_reader(keys);
    let broad = PermissionRequest { blocked_path: Some("/etc/hosts".into()), ..request("Bash", "touch /etc/hosts") };
    ask(&mut c, &broad);
    assert_eq!(*offered.lock().unwrap(), vec![false]);
    assert!(err.text().contains("can't remember this one: the folder it touches is too broad"), "{}", err.text());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_console_saves_nothing_and_prints_the_answer_only_once_the_engine_has_settled_it() {
    let dir = tmp_project("settled");
    let (mut c, err) = console_in(&dir, AnswerKey::Always);
    let req = blocked_touch("touch /tmp/spike/outside.txt");
    let offer = offer_for(&c, &req);
    assert_eq!(wait(c.ask_permission(&req, &offer)), Some(Decision::AllowAlways));
    assert!(!err.text().contains('\u{2713}'), "no answer line before the save: {}", err.text());
    assert!(!crate::approval::domain::settings_document::local_settings_path(&dir).exists(), "the console saves nothing itself");
    // The engine could not write the file: the screen says so, never "always".
    c.permission_settled(&req, &Settled::NotSaved(settings_document::SettingsError("settings: cannot write it".into())));
    let e = err.text();
    assert!(e.contains("✓ allowed\n") && e.contains("nothing was saved") && !e.contains("always in this project:"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_that_cannot_be_saved_still_allows_once_and_says_so() {
    let dir = tmp_project("blocked-fail");
    std::fs::write(dir.join(".claude"), "x").unwrap();
    let (mut c, err) = console_in(&dir, AnswerKey::Always);
    let d = ask(&mut c, &blocked_touch("touch /tmp/spike/outside.txt"));
    assert_eq!(d, Some(Decision::Allow));
    let e = err.text();
    assert!(e.contains("[permissions]") && e.contains("allowed this once, nothing was saved"), "{e}");
    assert!(!e.contains("always in this project:"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}
