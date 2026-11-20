//! `golden!("turn/turn_log_row.txt")` embeds `tests/golden/turn/turn_log_row.txt` as a `&'static str`.

/// The one place that knows where `tests/golden/` lives relative to the crate root.
/// When the crate moves two levels down, change the prefix below to `/../../tests/golden/`.
macro_rules! golden {
    ($path:expr) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/", $path))
    };
}

/// Replaces each value in `pairs` (`(value, token)`) in `text` by its fixed token, in order.
/// Used to hide per-run values (timestamps, process ids, temp paths) before comparing to a golden.
pub fn mask(text: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = text.to_string();
    for (value, token) in pairs {
        if !value.is_empty() {
            out = out.replace(value, token);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::mask;

    #[test]
    fn mask_replaces_every_occurrence() {
        let got = mask("ts=123 pid=42 ts=123", &[("123", "<TS>"), ("42", "<PID>")]);
        assert_eq!(got, "ts=<TS> pid=<PID> ts=<TS>");
    }

    #[test]
    fn mask_without_pairs_or_with_empty_value_is_identity() {
        assert_eq!(mask("abc", &[]), "abc");
        assert_eq!(mask("abc", &[("", "<X>")]), "abc");
    }

    #[test]
    fn mask_leaves_text_without_the_value() {
        assert_eq!(mask("abc", &[("zzz", "<X>")]), "abc");
    }
}
