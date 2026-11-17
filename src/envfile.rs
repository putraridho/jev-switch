//! Secrets lookup shared by every module that needs a key. An installed binary runs
//! from any directory without sourcing anything, so a key may live in the process
//! environment or in a `KEY=VALUE` file (`$JEV_ROUTER_ENV`, else `~/.jev-router.env`).
//! Parsing is pure ([`parse_any`]) and separate from the file IO.

use std::path::PathBuf;

/// The first non-empty value for any of `names`, reading `text` as `KEY=VALUE`
/// lines. Accepts a leading `export `, trims whitespace and one layer of quotes,
/// and skips lines whose value is empty. File order decides between several names.
pub fn parse_any(text: &str, names: &[&str]) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim().trim_start_matches("export ").trim();
        let (key, value) = line.split_once('=')?;
        names
            .contains(&key.trim())
            .then(|| value.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
            .filter(|v| !v.is_empty())
    })
}

/// [`parse_any`] for a single name.
pub fn parse(text: &str, name: &str) -> Option<String> {
    parse_any(text, &[name])
}

/// Where the key file lives: `$JEV_ROUTER_ENV` wins, else `<home>/.jev-router.env`.
/// Inputs are passed in so the choice is testable without touching the real
/// environment.
pub fn env_file_path(jev_router_env: Option<String>, home: Option<String>) -> Option<PathBuf> {
    jev_router_env.map(PathBuf::from).or_else(|| home.map(|h| PathBuf::from(format!("{h}/.jev-router.env"))))
}

/// A non-empty environment variable. Empty counts as unset so a blank `export X=`
/// cannot shadow a real key in the file.
pub fn from_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The first value for any of `names` in the key file; `None` when the file is
/// missing or has none of them.
pub fn from_file_any(names: &[&str]) -> Option<String> {
    let path = env_file_path(std::env::var("JEV_ROUTER_ENV").ok(), std::env::var("HOME").ok())?;
    parse_any(&std::fs::read_to_string(path).ok()?, names)
}

/// `name` from the environment first, then from the key file.
pub fn lookup(name: &str) -> Option<String> {
    from_env(name).or_else(|| from_file_any(&[name]))
}

/// A number setting: `name` from the environment or the key file, parsed; `default` when
/// it is unset or not a number.
pub fn lookup_f64(name: &str, default: f64) -> f64 {
    parse_f64(lookup(name).as_deref(), default)
}

/// The pure half of [`lookup_f64`].
pub fn parse_f64(value: Option<&str>, default: f64) -> f64 {
    value.and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_settings_fall_back_to_the_default_when_unset_or_not_a_number() {
        assert_eq!(parse_f64(None, 0.85), 0.85);
        assert_eq!(parse_f64(Some(" 0.9 "), 0.85), 0.9);
        assert_eq!(parse_f64(Some("2"), 0.0), 2.0);
        for bad in ["", "abc", "NaN", "inf"] {
            assert_eq!(parse_f64(Some(bad), 7.0), 7.0, "{bad:?}");
        }
    }

    #[test]
    fn parses_plain_export_and_quoted_lines() {
        let text = "A=1\nexport B = \"two\"\n  C='three'  \n";
        assert_eq!(parse(text, "A").as_deref(), Some("1"));
        assert_eq!(parse(text, "B").as_deref(), Some("two"));
        assert_eq!(parse(text, "C").as_deref(), Some("three"));
    }

    #[test]
    fn missing_and_empty_values_are_none() {
        assert_eq!(parse("A=1\n", "B"), None);
        assert_eq!(parse("A=\nB=\"\"\n", "A"), None);
        assert_eq!(parse("A=\nB=\"\"\n", "B"), None);
        assert_eq!(parse("", "A"), None);
        assert_eq!(parse("no equals sign\n# A=1\n", "A"), None);
    }

    #[test]
    fn empty_line_is_skipped_and_search_continues() {
        assert_eq!(parse("A=\nA=2\n", "A").as_deref(), Some("2"));
    }

    #[test]
    fn name_must_match_exactly() {
        assert_eq!(parse("MY_A=1\nA_B=2\n", "A"), None);
    }

    #[test]
    fn value_keeps_inner_equals_signs() {
        assert_eq!(parse("A=x=y\n", "A").as_deref(), Some("x=y"));
    }

    #[test]
    fn any_of_several_names_takes_the_first_in_file_order() {
        let text = "JEV_API_KEY=second\nTYPESAFE_API_KEY=first\n";
        assert_eq!(parse_any(text, &["TYPESAFE_API_KEY", "JEV_API_KEY"]).as_deref(), Some("second"));
    }

    #[test]
    fn env_file_path_prefers_the_override_then_home() {
        assert_eq!(env_file_path(Some("/x/e".into()), Some("/h".into())), Some(PathBuf::from("/x/e")));
        assert_eq!(env_file_path(None, Some("/h".into())), Some(PathBuf::from("/h/.jev-router.env")));
        assert_eq!(env_file_path(None, None), None);
    }
}
