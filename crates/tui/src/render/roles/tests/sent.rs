use super::*;

    #[test]
    fn a_sent_message_is_a_shaded_band_or_a_plain_marker_without_colour() {
        assert_eq!(sent_block("fix it", true, Some("6:27 PM"), OFF, 40), "❯ fix it\n", "no stamp without colour");
        assert_eq!(sent_block("one\ntwo", false, None, OFF, 20), format!("{}\n❯ one\n  two\n", "─".repeat(19)));
        let band = sent_block("x", false, Some("6:27 PM"), ON, 40);
        let rows: Vec<&str> = band.lines().collect();
        assert!(rows[0].contains("\x1b[48;5;236") && strip_ansi(rows[0]).trim().is_empty(), "a blank band row above the text sets it apart");
        assert!(strip_ansi(rows[1]).starts_with("  ❯ x") && strip_ansi(rows[1]).ends_with("6:27 PM  "), "{:?}", strip_ansi(rows[1]));
        assert!(strip_ansi(rows[2]).trim().is_empty() && rows[2].contains("\x1b[48;5;236"), "and below it");
        for row in &rows[..3] {
            assert_eq!(crate::render::display_width(&strip_ansi(row)), 39);
        }
        let long = sent_block(&"word ".repeat(20), true, Some("6:27 PM"), ON, 40);
        assert!(long.lines().all(|r| crate::render::display_width(&strip_ansi(r)) <= 39), "wrapped clear of the stamp: {long}");
    }


    #[test]
    fn a_route_row_leads_the_turn_with_the_provider_and_why() {
        let f = RouteFacts { provider: "codex".into(), model: "gpt-6.1-sol".into(), reason: "task: feature (0.9) → codex gpt-6.1-sol; first turn".into(), ..RouteFacts::default() };
        assert_eq!(route_row(&f, OFF, 80), "  ⇄ codex · gpt-6.1-sol   feature · first turn");
        assert!(route_row(&f, ON, 80).contains("\x1b[32mcodex"), "codex in its colour");
    }
