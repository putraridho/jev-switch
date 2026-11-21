use super::*;

#[test]
fn side_by_side_pairs_old_and_new_and_fits_the_width() {
    let out = render_diff(&parse(DIFF), &opts(100, Layout::Split), OFF);
    let l = lines(&out);
    let pair = l.iter().find(|l| l.contains("price * 3")).unwrap();
    assert!(
        pair.contains("price * 3;") && pair.contains("│") && pair.contains("price * quantity;"),
        "{pair}"
    );
    let extra = l.iter().find(|l| l.contains("let extra")).unwrap();
    assert!(
        extra.find('│').unwrap() < extra.find("let extra").unwrap(),
        "an added line with no old side leaves the left empty: {extra}"
    );
    assert!(l.iter().all(|l| display_width(l) <= 100), "{out}");
}

#[test]
fn auto_layout_goes_side_by_side_only_when_wide() {
    let narrow = render_diff(
        &parse(DIFF),
        &DiffOpts {
            width: 80,
            layout: Layout::Auto,
            fold: Fold::Open,
        },
        OFF,
    );
    let wide = render_diff(
        &parse(DIFF),
        &DiffOpts {
            width: 140,
            layout: Layout::Auto,
            fold: Fold::Open,
        },
        OFF,
    );
    assert!(!narrow.contains('│') && wide.contains('│'));
}
