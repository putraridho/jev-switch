use super::*;

#[test]
fn every_accent_is_readable_on_dark_and_light_terminals() {
    for palette in [Palette::Standard, Palette::ColorBlind] {
        for p in PROVIDERS {
            let a = builtin_identity(p, palette).accent;
            assert!(a.contrast(Rgb(0, 0, 0)) >= 4.5, "{p} on black");
            assert!(a.contrast(Rgb(255, 255, 255)) >= 2.0, "{p} on white");
        }
    }
}

#[test]
fn providers_differ_by_glyph_and_badge_not_only_colour() {
    let ids: Vec<Identity> = PROVIDERS.iter().map(|p| builtin_identity(p, Palette::Standard)).collect();
    for (i, a) in ids.iter().enumerate() {
        for b in &ids[i + 1..] {
            assert_ne!(a.glyph, b.glyph);
            assert_ne!(a.badge, b.badge);
        }
    }
    for palette in [Palette::Standard, Palette::ColorBlind] {
        let colours: Vec<Rgb> = PROVIDERS[..5].iter().map(|p| builtin_identity(p, palette).accent).collect();
        for (i, a) in colours.iter().enumerate() {
            assert!(colours[i + 1..].iter().all(|b| a != b));
        }
    }
    assert_eq!(builtin_identity("mystery-provider", Palette::Standard).badge, "mystery-");
}

#[test]
fn palette_names() {
    assert_eq!(Palette::parse("Colour-Blind"), Some(Palette::ColorBlind));
    assert_eq!(Palette::parse("default"), Some(Palette::Standard));
    assert_eq!(Palette::parse("neon"), None);
}
