use super::*;

#[test]
fn depth_detection_table() {
    for (ct, term, want) in [
        (Some("truecolor"), Some("xterm"), ColorDepth::TrueColor),
        (Some("24bit"), None, ColorDepth::TrueColor),
        (None, Some("xterm-256color"), ColorDepth::Ansi256),
        (None, Some("screen"), ColorDepth::Ansi16),
        (None, None, ColorDepth::Ansi16),
    ] {
        assert_eq!(ColorDepth::resolve(ct, term), want);
    }
}

#[test]
fn colour_sequences_per_depth() {
    let orange = Rgb(255, 135, 0);
    assert_eq!(orange.sgr(ColorDepth::TrueColor), "38;2;255;135;0");
    assert_eq!(orange.sgr(ColorDepth::Ansi256), "38;5;208");
    assert_eq!(Rgb(0, 205, 0).sgr(ColorDepth::Ansi16), "32");
    assert_eq!(Rgb(255, 0, 0).sgr(ColorDepth::Ansi16), "91");
}

#[test]
fn hex_parsing() {
    assert_eq!(Rgb::parse_hex("#ff8700"), Some(Rgb(255, 135, 0)));
    assert_eq!(Rgb::parse_hex("00ff00"), Some(Rgb(0, 255, 0)));
    for bad in ["", "#fff", "#gggggg", "#ff87000", "ééé"] {
        assert_eq!(Rgb::parse_hex(bad), None, "{bad}");
    }
}
