use super::super::cooldowns::{Cooldowns, DiskWatch};
use super::super::{DEFAULT_MS, MINUTE_MS};
use super::{never, temp_dir, unresolvable, NOW};
use crate::shared::ProviderId;
use ProviderId::{Claude, Codex};

#[test]
fn active_counts_down_then_expires() {
    let mut c = Cooldowns::default();
    assert_eq!(c.active(Claude, NOW), None);
    c.start(Claude, None, NOW);
    assert_eq!(c.active(Claude, NOW), Some(DEFAULT_MS));
    assert_eq!(c.active(Claude, NOW + DEFAULT_MS - 1), Some(1));
    assert_eq!(c.active(Claude, NOW + DEFAULT_MS), None);
    assert_eq!(c.active(Codex, NOW), None);
}

#[test]
fn start_with_uses_the_resolver() {
    let mut c = Cooldowns::default();
    c.start_with(Codex, Some("20m"), NOW, &never);
    assert_eq!(c.active(Codex, NOW), Some(20 * MINUTE_MS));
}

#[test]
fn clear_removes() {
    let mut c = Cooldowns::default();
    c.start(Claude, None, NOW);
    c.clear(Claude);
    assert_eq!(c.active(Claude, NOW), None);
}

#[test]
fn json_round_trip_and_garbage() {
    let mut c = Cooldowns::default();
    c.start(Claude, None, NOW);
    c.start(Codex, Some(&(NOW / 1000 + 7200).to_string()), NOW);
    assert_eq!(Cooldowns::from_json(&c.to_json()), c);
    for garbage in [serde_json::json!(null), serde_json::json!([1, 2]), serde_json::json!("x")] {
        assert_eq!(Cooldowns::from_json(&garbage), Cooldowns::default());
    }
    let partly = serde_json::json!({"claude": 5, "nobody": 6, "codex": "soon", "gemini": -1});
    let c = Cooldowns::from_json(&partly);
    assert_eq!(c.until_ms.len(), 1);
    assert_eq!(c.until_ms[&Claude], 5);
}

fn with(entries: &[(ProviderId, u64)]) -> Cooldowns {
    Cooldowns { until_ms: entries.iter().copied().collect() }
}

#[test]
fn save_then_load_survives_a_restart_and_leaves_no_temp_file() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("cooldowns.json");
    let mut c = Cooldowns::default();
    c.start(Claude, None, NOW);
    c.save(&path, NOW).unwrap();
    assert_eq!(Cooldowns::load(&path), c);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_merges_with_the_file_keeping_the_later_expiry() {
    let dir = temp_dir("merge");
    let path = dir.join("cooldowns.json");
    with(&[(Claude, NOW + 5000), (Codex, NOW + 9000)]).save(&path, NOW).unwrap();
    // Another process knows a later Claude limit but nothing about Codex.
    let written = with(&[(Claude, NOW + 8000)]).save(&path, NOW).unwrap();
    assert_eq!(written, with(&[(Claude, NOW + 8000), (Codex, NOW + 9000)]));
    assert_eq!(Cooldowns::load(&path), written);
    // An earlier expiry never shortens what is on disk.
    with(&[(Claude, NOW + 100)]).save(&path, NOW).unwrap();
    assert_eq!(Cooldowns::load(&path).until_ms[&Claude], NOW + 8000);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_drops_expired_entries_and_survives_a_garbage_file() {
    let dir = temp_dir("expired");
    let path = dir.join("cooldowns.json");
    std::fs::write(&path, "{not json").unwrap();
    let written = with(&[(Claude, NOW - 1), (Codex, NOW + 1)]).save(&path, NOW).unwrap();
    assert_eq!(written, with(&[(Codex, NOW + 1)]));
    assert_eq!(Cooldowns::load(&path), written);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_tolerates_missing_and_corrupt_files() {
    let dir = temp_dir("corrupt");
    assert_eq!(Cooldowns::load(&dir.join("absent.json")), Cooldowns::default());
    let bad = dir.join("bad.json");
    std::fs::write(&bad, "{not json").unwrap();
    assert_eq!(Cooldowns::load(&bad), Cooldowns::default());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn disk_watch_reads_only_when_the_file_changed() {
    let dir = temp_dir("watch");
    let path = dir.join("cooldowns.json");
    let mut w = DiskWatch::default();
    // Missing file: empty, and the initial state already matches it, so no read.
    assert_eq!(*w.refresh(&path), Cooldowns::default());
    assert_eq!(w.reloads(), 0);

    with(&[(Claude, NOW + 1)]).save(&path, NOW).unwrap();
    assert_eq!(*w.refresh(&path), with(&[(Claude, NOW + 1)]));
    assert_eq!(w.reloads(), 1);
    for _ in 0..3 {
        w.refresh(&path);
    }
    assert_eq!(w.reloads(), 1, "unchanged file: no reload");

    // A change in size is seen even if the clock did not tick.
    with(&[(Claude, NOW + 1), (Codex, NOW + 2)]).save(&path, NOW).unwrap();
    assert_eq!(w.refresh(&path).until_ms.len(), 2);
    assert_eq!(w.reloads(), 2);

    // Garbage and deletion are tolerated and read as empty.
    std::fs::write(&path, "garbage that is longer than before").unwrap();
    assert_eq!(*w.refresh(&path), Cooldowns::default());
    std::fs::remove_file(&path).unwrap();
    assert_eq!(*w.refresh(&path), Cooldowns::default());
    assert_eq!(w.reloads(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn golden_cooldowns() {
    let dir = std::env::temp_dir().join(format!("jev-golden-cooldowns-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let saved = |name: &str, before: Option<&str>, cd: &Cooldowns| {
        let path = dir.join(name);
        if let Some(text) = before {
            std::fs::write(&path, text).unwrap();
        }
        cd.save(&path, NOW).unwrap();
        std::fs::read_to_string(&path).unwrap()
    };
    let mut typical = Cooldowns::default();
    typical.start_with(Claude, Some("1760003600"), NOW, &never);
    typical.start_with(Codex, None, NOW, &unresolvable);
    // The file already holds an expired entry, a later one for the same provider, a number
    // that is not a time and a provider nobody knows: the save merges and drops what is over.
    let mut odd = Cooldowns::default();
    odd.start_with(Claude, Some("a quarter of an hour"), NOW, &unresolvable);
    let before = r#"{"claude": 1760000000001, "codex": 1759999999999, "gemini": 1760090000000, "grok": "soon", "nobody": 5}"#;
    let actual = format!(
        "## typical: two providers cooling down\n{}\n## unusual: merged into an existing file with expired, later and unreadable entries\n{}\n## unusual: nothing cooling down\n{}\n",
        saved("a.json", None, &typical),
        saved("b.json", Some(before), &odd),
        saved("c.json", None, &Cooldowns::default()),
    );
    let _ = std::fs::remove_dir_all(&dir);
    let golden = golden!("providers/cooldowns.txt");
    assert_eq!(golden.split_once('\n').unwrap().1, actual);
}
