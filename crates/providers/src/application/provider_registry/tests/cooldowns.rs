//! Cooldowns: persisting them, and seeing other processes' limits.

use std::path::PathBuf;

use super::*;
use crate::domain::reset_time::cooldowns::Cooldowns;

    #[test]
    fn cool_down_persists_to_the_given_path() {
        let path = std::env::temp_dir().join(format!("jev-providers-test-{}.json", std::process::id()));
        let mut rt = runtime();
        rt.cooldown_path = Some(path.clone());
        rt.cool_down(Codex, Some("1760003600"), NOW);
        let loaded = rt.host.load_cooldowns(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(loaded.active(Codex, NOW), Some(3_600_000));
        assert!(rt.cooldowns.active(Codex, NOW).is_some());
    }

    fn shared_path(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("jev-providers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cooldowns.json");
        (dir, path)
    }

    #[test]
    fn two_processes_see_each_others_cooldowns_without_restarting() {
        let (dir, path) = shared_path("two");
        let (mut a, mut b) = (runtime(), runtime());
        a.cooldown_path = Some(path.clone());
        b.cooldown_path = Some(path.clone());
        assert!(a.usable(NOW).contains(some_model(Claude)));

        a.cool_down(Codex, None, NOW);
        b.cool_down(Claude, None, NOW);

        // The file holds both, because B's save merged with A's.
        let on_disk = a.host.load_cooldowns(&path);
        assert!(on_disk.active(Codex, NOW).is_some() && on_disk.active(Claude, NOW).is_some());
        // A never cooled Claude itself, yet stops offering it, and still skips Codex.
        let usable = a.usable(NOW + 1);
        assert!(!usable.contains(some_model(Claude)));
        assert!(!usable.contains(some_model(Codex)));
        // B's own set picked up A's Codex limit through the merge.
        assert!(b.cooldowns.active(Codex, NOW).is_some());
        assert!(!b.usable(NOW + 1).contains(some_model(Codex)));
        // Once the limits pass, both are back.
        assert!(a.usable(NOW + 2 * 60 * MINUTE_MS).contains(some_model(Claude)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usable_reads_the_cooldown_file_only_when_it_changed() {
        let (dir, path) = shared_path("mtime");
        let mut a = runtime();
        a.cooldown_path = Some(path.clone());
        let reloads = |rt: &Runtime| rt.disk.lock().unwrap().reloads();
        a.usable(NOW);
        a.usable(NOW);
        assert_eq!(reloads(&a), 0, "no file, nothing to read");

        let mut other = Cooldowns::default();
        other.start_in(Claude, None, NOW, &|_, _| None, &|_| false);
        a.host.save_cooldowns(&path, &other, NOW).unwrap();
        assert!(!a.usable(NOW + 1).contains(some_model(Claude)));
        assert_eq!(reloads(&a), 1);
        for _ in 0..5 {
            a.usable(NOW + 1);
        }
        assert_eq!(reloads(&a), 1, "unchanged file: no reload");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_garbage_cooldown_file_is_tolerated_by_usable_and_cool_down() {
        let (dir, path) = shared_path("garbage");
        std::fs::write(&path, "{ definitely not json").unwrap();
        let mut a = runtime();
        a.cooldown_path = Some(path.clone());
        assert!(a.usable(NOW).contains(some_model(Claude)));
        a.cool_down(Claude, None, NOW);
        assert!(!a.usable(NOW + 1).contains(some_model(Claude)));
        assert!(a.host.load_cooldowns(&path).active(Claude, NOW).is_some(), "rewritten as valid json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cool_down_without_a_path_still_cools() {
        let mut rt = runtime();
        rt.cool_down(Claude, None, NOW);
        assert!(rt.cooldowns.active(Claude, NOW).is_some());
    }
