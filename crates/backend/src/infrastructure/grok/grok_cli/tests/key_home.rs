use std::path::{Path, PathBuf};

use super::super::key_home::{needs_mirror, real_home};
#[cfg(unix)]
use super::super::key_home::sync_mirror;

fn scratch(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("jev-grok-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn key() -> Vec<(String, String)> {
    vec![("XAI_API_KEY".into(), "k".into())]
}

#[test]
fn the_mirror_is_needed_only_when_a_key_meets_a_saved_login() {
    let real = scratch("need");
    assert!(!needs_mirror(&key(), &real), "no login: the key is used anyway");
    std::fs::write(real.join("auth.json"), "{}").unwrap();
    assert!(needs_mirror(&key(), &real));
    assert!(!needs_mirror(&[], &real), "no key: the login is all there is");
    std::fs::remove_dir_all(&real).unwrap();
}

#[test]
fn real_home_follows_grok_home_else_dot_grok() {
    let home = Path::new("/h");
    assert_eq!(real_home(&|_| None, Some(home)), Some(PathBuf::from("/h/.grok")));
    assert_eq!(real_home(&|n| (n == "GROK_HOME").then(|| "/g".into()), Some(home)), Some(PathBuf::from("/g")));
    assert_eq!(real_home(&|_| Some(" ".into()), None), None);
}

#[cfg(unix)]
#[test]
fn the_mirror_links_everything_but_the_login_and_writes_reach_the_real_home() {
    let real = scratch("real");
    let mirror = scratch("mirror").join("m");
    std::fs::write(real.join("auth.json"), "{}").unwrap();
    std::fs::write(real.join("auth.json.commit.lock"), "").unwrap();
    std::fs::write(real.join("config.toml"), "x = 1").unwrap();
    std::fs::create_dir(real.join("sessions")).unwrap();

    sync_mirror(&real, &mirror).unwrap();
    assert_eq!(names(&mirror), ["config.toml", "sessions"]);
    std::fs::write(mirror.join("sessions/s1"), "turn").unwrap();
    assert_eq!(std::fs::read_to_string(real.join("sessions/s1")).unwrap(), "turn");

    // Later: Grok adds an entry, one goes away, and a login gets written into the mirror.
    std::fs::create_dir(real.join("skills")).unwrap();
    std::fs::remove_file(real.join("config.toml")).unwrap();
    std::fs::write(mirror.join("auth.json"), "{}").unwrap();
    sync_mirror(&real, &mirror).unwrap();
    assert_eq!(names(&mirror), ["sessions", "skills"]);
    assert!(real.join("auth.json").exists(), "the user's own login is never touched");

    std::fs::remove_dir_all(&real).unwrap();
    std::fs::remove_dir_all(mirror.parent().unwrap()).unwrap();
}
