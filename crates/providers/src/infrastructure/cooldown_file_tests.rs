use super::{CooldownFile, Cooldowns};
use crate::application::provider_registry::disk_watch::DiskWatch;
use crate::infrastructure::probe_facts::SystemProbes;
use jev_shared::ProviderId;
use ProviderId::{Claude, Codex};

const NOW: u64 = 1_760_000_000_000;

fn never(_: &str, _: Option<&str>) -> Option<u64> { panic!("resolver must not be called") }
fn unresolvable(_: &str, _: Option<&str>) -> Option<u64> { None }
fn temp_dir(name: &str) -> std::path::PathBuf { jev_testkit::scratch_dir::scratch_dir("cooldown", name) }

trait TestFile {
    fn load(path: &std::path::Path) -> Self;
    fn save(&self, path: &std::path::Path, now_ms: u64) -> std::io::Result<Self> where Self: Sized;
    fn to_json(&self) -> serde_json::Value;
    fn from_json(value: &serde_json::Value) -> Self;
}

impl TestFile for Cooldowns {
    fn load(path: &std::path::Path) -> Self { super::load(path) }
    fn save(&self, path: &std::path::Path, now_ms: u64) -> std::io::Result<Self> { super::save(path, self, now_ms) }
    fn to_json(&self) -> serde_json::Value { CooldownFile::from(self).0 }
    fn from_json(value: &serde_json::Value) -> Self { CooldownFile(value.clone()).into() }
}

#[test]
fn json_round_trip_and_garbage() {
    let mut c = Cooldowns::default();
    c.start_in(Claude, None, NOW, &never, &|_| false);
    c.start_in(Codex, Some(&(NOW / 1000 + 7200).to_string()), NOW, &never, &|_| false);
    assert_eq!(Cooldowns::from_json(&c.to_json()), c);
    for garbage in [serde_json::json!(null), serde_json::json!([1, 2]), serde_json::json!("x")] {
        assert_eq!(Cooldowns::from_json(&garbage), Cooldowns::default());
    }
    let partly = serde_json::json!({"claude": 5, "nobody": 6, "codex": "soon", "gemini": -1});
    let c = Cooldowns::from_json(&partly);
    assert_eq!(c.entries().len(), 1);
    assert_eq!(c.entries()[&Claude], 5);
}

fn with(entries: &[(ProviderId, u64)]) -> Cooldowns {
    entries.iter().copied().collect::<std::collections::HashMap<_, _>>().into()
}

#[test]
fn save_then_load_survives_a_restart_and_leaves_no_temp_file() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("cooldowns.json");
    let mut c = Cooldowns::default();
    c.start_in(Claude, None, NOW, &never, &|_| false);
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
    assert_eq!(Cooldowns::load(&path).entries()[&Claude], NOW + 8000);
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
    assert_eq!(*w.refresh(&path, &SystemProbes), Cooldowns::default());
    assert_eq!(w.reloads(), 0);

    with(&[(Claude, NOW + 1)]).save(&path, NOW).unwrap();
    assert_eq!(*w.refresh(&path, &SystemProbes), with(&[(Claude, NOW + 1)]));
    assert_eq!(w.reloads(), 1);
    for _ in 0..3 {
        w.refresh(&path, &SystemProbes);
    }
    assert_eq!(w.reloads(), 1, "unchanged file: no reload");

    // A change in size is seen even if the clock did not tick.
    with(&[(Claude, NOW + 1), (Codex, NOW + 2)]).save(&path, NOW).unwrap();
    assert_eq!(w.refresh(&path, &SystemProbes).entries().len(), 2);
    assert_eq!(w.reloads(), 2);

    // Garbage and deletion are tolerated and read as empty.
    std::fs::write(&path, "garbage that is longer than before").unwrap();
    assert_eq!(*w.refresh(&path, &SystemProbes), Cooldowns::default());
    std::fs::remove_file(&path).unwrap();
    assert_eq!(*w.refresh(&path, &SystemProbes), Cooldowns::default());
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
    typical.start_in(Claude, Some("1760003600"), NOW, &never, &|_| false);
    typical.start_in(Codex, None, NOW, &unresolvable, &|_| false);
    // The file already holds an expired entry, a later one for the same provider, a number
    // that is not a time and a provider nobody knows: the save merges and drops what is over.
    let mut odd = Cooldowns::default();
    odd.start_in(Claude, Some("a quarter of an hour"), NOW, &unresolvable, &|_| false);
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
