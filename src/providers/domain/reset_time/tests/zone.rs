use std::cell::RefCell;

use super::super::{cooldown_ms_in, cooldown_ms_with, DEFAULT_MS, HOUR_MS};
use super::{cooldown_in, fixed, never, unresolvable, NOW};

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
}
