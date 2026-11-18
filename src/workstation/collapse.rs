//! Folded tool output: the first lines of a long output and a note saying how many more
//! there are and how to see them.

/// `lines` cut to `keep` rows when there are more than `keep + 1` (a fold that hides one
/// line would say as much as the line). The note row is the last one.
pub fn fold(lines: &[String], keep: usize, expand_hint: &str) -> Vec<String> {
    if lines.len() <= keep + 1 {
        return lines.to_vec();
    }
    let mut out: Vec<String> = lines[..keep].to_vec();
    out.push(format!("… +{} lines ({expand_hint})", lines.len() - keep));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("l{i}")).collect()
    }

    #[test]
    fn short_output_stays_whole_and_long_output_folds_with_a_count() {
        assert_eq!(fold(&n(3), 3, "x"), n(3));
        assert_eq!(fold(&n(4), 3, "x"), n(4), "a fold of one line saves nothing");
        let f = fold(&n(10), 3, "/expand");
        assert_eq!(f.len(), 4);
        assert_eq!(f[3], "… +7 lines (/expand)");
        assert_eq!(fold(&[], 3, "x"), Vec::<String>::new());
    }
}
