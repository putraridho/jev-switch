use super::*;

    #[test]
    fn tables_are_aligned_in_columns() {
        let t = "| name | n |\n|:--|--:|\n| alpha | 1 |\n| b | 100 |";
        let out = answer_text(t, ON, 80);
        assert_eq!(strip_ansi(&out), "name     n\n─────  ───\nalpha    1\nb      100");
        assert!(out.starts_with("\x1b[1mname\x1b[0m"), "{out:?}");
        assert!(out.contains("\x1b[2m─────  ───\x1b[0m"), "{out:?}");
        // wide characters count by columns, centre alignment splits the gap
        let wide = answer_text("| a | b |\n|:-:|---|\n| 日本 | x |", ON, 80);
        assert_eq!(strip_ansi(&wide), " a    b\n────  ─\n日本  x");
        // prose around a table is kept, and a short row is padded
        let mixed = answer_text("before\n| a | b |\n|---|---|\n| 1 |\nafter", ON, 80);
        assert_eq!(strip_ansi(&mixed), "before\na  b\n─  ─\n1\nafter");
    }

    #[test]
    fn tables_too_wide_or_malformed_are_left_alone() {
        let wide = "| a | b |\n|---|---|\n| a very long cell that does not fit | x |";
        assert_eq!(answer_text(wide, ON, 20), wide);
        for t in ["| a | b |", "| a | b |\n| 1 | 2 |", "| a | b |\n|---|\n| 1 | 2 |", "| a |\n|---|\n| 1 | 2 | 3 |", "|\n|"] {
            assert_eq!(answer_text(t, ON, 80), t, "{t:?}");
        }
    }
