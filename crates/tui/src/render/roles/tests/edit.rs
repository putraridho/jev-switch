use super::*;

    #[test]
    fn an_edit_with_a_known_start_numbers_its_lines_and_tints_them_with_colour() {
        let removed = vec!["let a = 1;".to_string()];
        let added = vec!["let a = 2;".to_string(), "let b = 3;".to_string()];
        assert_eq!(edit_preview_at(&removed, &added, Some(9), OFF, 80), "    ⎿ +2 −1\n       9 - let a = 1;\n       9 + let a = 2;\n      10 + let b = 3;");
        let on = edit_preview_at(&removed, &added, Some(9), ON, 40);
        assert!(on.contains("\x1b[48;5;52") && on.contains("\x1b[48;5;22"), "{on:?}");
        for row in on.lines().skip(1) {
            assert!(crate::render::display_width(&strip_ansi(row)) <= 40, "{row:?}");
        }
        assert_eq!(edit_preview_at(&removed, &added, None, OFF, 80), edit_preview(&removed, &added, OFF, 80));
    }


    #[test]
    fn an_edit_shows_a_small_diff_and_counts_the_rest() {
        let removed = vec!["let a = 1;".to_string()];
        let added = vec!["let a = 2;".to_string(), "let b = 3;".to_string()];
        assert_eq!(edit_preview(&removed, &added, OFF, 80), "    ⎿ +2 −1\n      - let a = 1;\n      + let a = 2;\n      + let b = 3;");
        let many: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let new_file = edit_preview(&[], &many, OFF, 80);
        assert!(new_file.starts_with("    ⎿ new file, 10 lines\n      + line 0"), "{new_file}");
        assert!(new_file.ends_with("… 4 more lines · /diff after the turn"), "{new_file}");
    }
