//! Accessibility switches read from the environment: reduced motion and the colourless theme.

use std::sync::OnceLock;

/// Whether to keep the screen still: `JEV_REDUCED_MOTION` (or the common `REDUCE_MOTION`)
/// set to anything but empty, `0`, `false` or `no`.
pub fn reduced_motion_from(jev: Option<&str>, common: Option<&str>) -> bool {
    [jev, common].into_iter().flatten().any(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no"))
}

/// [`reduced_motion_from`] for this process's environment, read once.
pub fn reduced_motion() -> bool {
    static REDUCED: OnceLock<bool> = OnceLock::new();
    *REDUCED.get_or_init(|| {
        let var = |k: &str| std::env::var(k).ok();
        reduced_motion_from(var("JEV_REDUCED_MOTION").as_deref(), var("REDUCE_MOTION").as_deref())
    })
}

/// Whether `JEV_THEME` asks for no colour (`mono`, also `none`, `plain`).
pub fn mono_theme(theme: Option<&str>) -> bool {
    theme.is_some_and(|t| matches!(t.trim().to_ascii_lowercase().as_str(), "mono" | "none" | "plain"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_motion_follows_either_variable_and_ignores_off_words() {
        assert!(!reduced_motion_from(None, None));
        assert!(reduced_motion_from(Some("1"), None));
        assert!(reduced_motion_from(None, Some("true")));
        for off in ["", "0", "false", "No"] {
            assert!(!reduced_motion_from(Some(off), None), "{off}");
        }
    }

    #[test]
    fn the_mono_theme_is_named_by_a_few_words() {
        assert!(mono_theme(Some("mono")) && mono_theme(Some(" Plain ")));
        assert!(!mono_theme(Some("dark")) && !mono_theme(None));
    }
}
