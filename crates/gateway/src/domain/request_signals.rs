//! Pure helpers over prompt text: the project directory Claude Code reports and prompt similarity.

const CWD_KEY: &str = "Primary working directory: ";

/// The directory Claude Code reports in a piece of text, if the text has that line.
pub fn cwd_in_text(t: &str) -> Option<String> {
    let line = t[t.find(CWD_KEY)? + CWD_KEY.len()..].lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

pub fn project_of(cwd: &str) -> &str {
    cwd.trim_end_matches('/').rsplit('/').next().unwrap_or(cwd)
}

/// Jaccard overlap of the word sets: near 1.0 means the same question asked again.
pub fn similarity(a: &str, b: &str) -> f64 {
    let words = |s: &str| -> std::collections::HashSet<String> {
        s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
    };
    let (a, b) = (words(a), words(b));
    let union = a.union(&b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(&b).count() as f64 / union as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rephrase_scores_high_new_task_low() {
        assert!(similarity("fix the upload hang", "fix upload hang please") > 0.5);
        assert!(similarity("fix the upload hang", "add a unit test") < 0.2);
        assert_eq!(similarity("", ""), 0.0);
    }
}
