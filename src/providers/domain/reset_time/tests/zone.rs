use std::cell::RefCell;

use super::super::resolver::{system_date, system_zone_exists, zone_in_dir, zone_is_known};
use super::super::wall_clock::{normalize_for_date, split_zone};
use super::super::{cooldown_ms_in, cooldown_ms_with, DEFAULT_MS, HOUR_MS};
use super::{cooldown_in, fixed, never, temp_dir, unresolvable, NOW};

#[test]
fn only_real_zones_are_accepted_abbreviations_and_unknown_names_are_the_default() {
    let seen = RefCell::new(vec![]);
    let two_hours = fixed(NOW / 1000 + 7200, &seen);
    // `TZ=PST date ...` exits 0 reading UTC, 8 hours wrong, so PST is not a zone.
    assert_eq!(cooldown_in(Some("5pm (PST)"), NOW, &two_hours), DEFAULT_MS);
    assert_eq!(cooldown_in(Some("5pm (Not/AZone)"), NOW, &two_hours), DEFAULT_MS);
    assert_eq!(cooldown_in(Some("5pm (EST)"), NOW, &two_hours), DEFAULT_MS);
    assert!(seen.borrow().is_empty(), "an unknown zone never reaches the resolver");
    assert_eq!(cooldown_in(Some("5pm (Asia/Jakarta)"), NOW, &two_hours), 2 * HOUR_MS);
    // Exactly UTC and GMT need no zone file.
    for zone in ["UTC", "GMT"] {
        assert_eq!(cooldown_ms_in(Some(&format!("5pm ({zone})")), NOW, &two_hours, &|_| false), 2 * HOUR_MS, "{zone}");
    }
    for zone in ["utc", "Utc", "UTC+3", "Z"] {
        assert_eq!(cooldown_ms_in(Some(&format!("5pm ({zone})")), NOW, &two_hours, &|_| false), DEFAULT_MS, "{zone}");
    }
    // No zone at all is local time and still goes to the resolver.
    assert_eq!(cooldown_in(Some("5pm"), NOW, &two_hours), 2 * HOUR_MS);
}

#[cfg(unix)]
#[test]
fn a_zone_must_be_a_tzif_file_inside_the_zone_directory() {
    let dir = temp_dir("zones");
    let root = dir.join("zoneinfo");
    std::fs::create_dir_all(root.join("Asia")).unwrap();
    std::fs::write(root.join("Asia/Jakarta"), b"TZif2 rest of the file").unwrap();
    std::fs::write(root.join("zone.tab"), b"# not a zone").unwrap();
    std::fs::write(dir.join("outside"), b"TZif2 but outside").unwrap();
    std::os::unix::fs::symlink(dir.join("outside"), root.join("Escape")).unwrap();
    std::os::unix::fs::symlink(root.join("Asia/Jakarta"), root.join("Alias")).unwrap();
    for ok in ["Asia/Jakarta", "Alias"] {
        assert!(zone_in_dir(&root, ok), "{ok}");
    }
    for bad in ["PST", "Not/AZone", "Asia", "zone.tab", "Escape", "../outside", "/etc/passwd", "", "Asia/../zone.tab"] {
        assert!(!zone_in_dir(&root, bad), "{bad}");
    }
    assert!(!zone_in_dir(&dir.join("missing"), "Asia/Jakarta"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_real_resolver_refuses_abbreviations_and_unknown_zones() {
    assert_eq!(system_date("5pm", Some("PST")), None);
    assert_eq!(system_date("5pm", Some("Not/AZone")), None);
}

#[test]
fn what_the_resolver_cannot_read_is_the_default() {
    assert_eq!(cooldown_in(Some("5pm (Asia/Jakarta)"), NOW, &unresolvable), DEFAULT_MS);
}

#[test]
fn unsafe_text_never_reaches_the_resolver_or_panics() {
    let long = "a".repeat(500);
    let bad = [
        "$(touch /tmp/pwned)",
        "`id`",
        "5pm; rm -rf /",
        "5pm && ls",
        "5pm | cat",
        "@1760000000",
        "\"5pm\"",
        "5pm\n6pm",
        "5pm (Asia/Jakarta; ls)",
        "5pm (../../etc/passwd)",
        "5pm (/etc/passwd)",
        "5pm (-x)",
        "5pm ()",
        "(Asia/Jakarta)",
        "5pm (Asia/Jakarta",
        "5pm )",
        "5pm (a) (b",
        "17 h\u{e9}",
        "\u{0}",
        &long,
    ];
    for text in bad {
        assert_eq!(cooldown_ms_with(Some(text), NOW, &never), DEFAULT_MS, "{text:?}");
    }
    // The real resolver refuses them too, without spawning anything.
    for text in bad {
        assert_eq!(system_date(text, None), None, "{text:?}");
    }
    assert_eq!(system_date("5pm", Some("../etc")), None);
    assert_eq!(system_date("5pm", Some("/etc/passwd")), None);
    assert_eq!(system_date("5pm", Some("")), None);
}

/// Runs the real GNU `date` once, on input that needs no time zone data; skipped
/// where `date` is missing or not GNU (no `-d`).
#[test]
fn the_real_resolver_runs_date_when_it_exists() {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let Some(at) = system_date("1 hour", None) else {
        eprintln!("skipped: no GNU date on this machine");
        return;
    };
    assert!((now + 3590..=now + 3700).contains(&at), "{at} vs {now}");
    // The forms Claude Code prints, after normalizing, are ones GNU date accepts.
    for text in ["5pm (Asia/Jakarta)", "Nov 20, 5pm (Asia/Jakarta)", "Nov 20 at 5:30pm (UTC)"] {
        let (body, zone) = split_zone(text).unwrap();
        if zone.is_some_and(|z| !zone_is_known(z, &system_zone_exists)) {
            eprintln!("skipped {text}: no zone database on this machine");
            continue;
        }
        assert!(system_date(&normalize_for_date(body), zone).is_some(), "{text}");
    }
}
