use super::*;

#[test]
fn long_files_fold_and_say_how_to_open_them() {
    let mut text =
        String::from("diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -0,0 +1,100 @@\n");
    for i in 0..100 {
        text.push_str(&format!("+line {i}\n"));
    }
    let files = parse(&text);
    let folded = render_diff(
        &files,
        &DiffOpts {
            width: 80,
            layout: Layout::Unified,
            fold: Fold::After(10),
        },
        OFF,
    );
    assert_eq!(folded.lines().count(), 1 + 10 + 1);
    assert!(
        folded.contains("⋯ 91 more lines folded (/diff --full to show)"),
        "{folded}"
    );
    let collapsed = render_diff(
        &files,
        &DiffOpts {
            width: 80,
            layout: Layout::Unified,
            fold: Fold::Collapsed,
        },
        OFF,
    );
    assert_eq!(collapsed.lines().count(), 2);
    assert_eq!(
        render_diff(&files, &opts(80, Layout::Unified), OFF)
            .lines()
            .count(),
        1 + 1 + 100
    );
}

#[test]
fn a_file_with_no_hunks_says_why() {
    let files = parse("diff --git a/i.png b/i.png\nBinary files a/i.png and b/i.png differ\n");
    assert!(render_diff(&files, &opts(80, Layout::Unified), OFF).contains("no text changes"));
}

#[test]
fn a_hunk_view_names_its_position_and_no_line_is_too_wide() {
    let files = parse(DIFF);
    let out = render_hunk(&files[0], 0, &opts(30, Layout::Unified), OFF).unwrap();
    assert!(out.starts_with("▸ src/a.rs  hunk 1 of 1"), "{out}");
    assert!(out.lines().all(|l| display_width(l) <= 30), "{out}");
    assert!(render_hunk(&files[0], 1, &opts(30, Layout::Unified), OFF).is_none());
}

#[test]
fn tabs_and_wide_characters_stay_inside_the_width() {
    let text = "diff --git a/m.go b/m.go\n--- a/m.go\n+++ b/m.go\n@@ -1 +1 @@\n-\tx := \"日本語日本語日本語\"\n+\ty := \"日本語日本語日本語\"\n";
    for layout in [Layout::Unified, Layout::Split] {
        let out = render_diff(&parse(text), &opts(40, layout), OFF);
        assert!(out.lines().all(|l| display_width(l) <= 40), "{out}");
    }
}
