//! The next-prompt suggestion: after a turn, the empty input box can show what you would
//! likely type next, dim, and Right puts it in the box. It comes from the answer itself: a
//! reply that ends by offering a next step ("Want me to commit this and tag v1.1.0?") is
//! answered with that step ("commit this and tag v1.1.0"). Nothing is sent anywhere, so it is
//! instant, free and outside the privacy policy's reach; no offer, no suggestion.

/// The longest suggestion shown: a longer offer is a paragraph, not a prompt.
const MAX_CHARS: usize = 90;

/// How far back from the end of the answer an offer is looked for: it is the closing line.
const TAIL_CHARS: usize = 600;

/// Lead-ins that offer a next step, lower case; what follows them is the step. Only phrases
/// that are an offer whatever surrounds them: "I can also see..." and "happy to report..."
/// are statements as often as offers, so they are not here.
const OFFERS: [&str; 10] = [
    "do you want me to ",
    "would you like me to ",
    "want me to ",
    "shall i ",
    "should i ",
    "if you want me to ",
    "if you'd like me to ",
    "if you would like me to ",
    "if you want, i can ",
    "if you'd like, i can ",
];

/// Words at the end of the step that only make it polite (`for you`, `now`).
const TRAILERS: [&str; 6] = [" for you", " next", " now", " as well", " too", " instead"];

/// The prompt to suggest after `answer`, or `None`.
pub fn next_prompt(answer: &str) -> Option<String> {
    let tail = tail(answer.trim_end(), TAIL_CHARS);
    // The last sentence that carries an offer wins: it is the one the answer ends on.
    let mut best: Option<String> = None;
    for sentence in sentences(tail) {
        if let Some(step) = offered_step(&sentence) {
            best = Some(step);
        }
    }
    best
}

/// The last `max` bytes of `s`, starting on a character boundary.
fn tail(s: &str, max: usize) -> &str {
    let mut from = s.len().saturating_sub(max);
    while !s.is_char_boundary(from) {
        from += 1;
    }
    &s[from..]
}

/// Sentences of plain text: markdown emphasis and list markers removed, split after `.`,
/// `?` or `!` followed by a space or a line break, and at line breaks.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches(['-', '*', '>', '#']).trim();
        let plain = line.replace("**", "").replace("__", "");
        let mut current = String::new();
        let mut chars = plain.chars().peekable();
        while let Some(c) = chars.next() {
            current.push(c);
            if matches!(c, '.' | '?' | '!') && chars.peek().is_none_or(|n| n.is_whitespace()) {
                out.push(std::mem::take(&mut current).trim().to_string());
            }
        }
        if !current.trim().is_empty() {
            out.push(current.trim().to_string());
        }
    }
    out
}

/// The step `sentence` offers, ready to send as a prompt.
fn offered_step(sentence: &str) -> Option<String> {
    let lower = sentence.to_lowercase();
    // The offer that starts latest: "If you want, I can also X" is "X", not "also X".
    let (at, lead) = OFFERS.iter().filter_map(|lead| lower.rfind(lead).map(|at| (at, *lead))).max_by_key(|(at, lead)| at + lead.len())?;
    // `to_lowercase` can change lengths outside ASCII; then the offsets are not safe to use.
    if lower.len() != sentence.len() {
        return None;
    }
    let mut step = sentence[at + lead.len()..].trim().to_string();
    // A question offering a choice: the first option.
    if let Some(i) = step.find(", or ") {
        step.truncate(i);
    }
    step = step.trim_end_matches(['?', '.', '!', ' ']).to_string();
    for trailer in TRAILERS {
        if step.to_lowercase().ends_with(trailer) {
            step.truncate(step.len() - trailer.len());
        }
    }
    let step = step.trim().trim_start_matches("also ").trim();
    // Model text drawn in the input box: anything that could drive the terminal is refused.
    let printable = !step.chars().any(char::is_control);
    let usable = printable && !step.is_empty() && step.chars().count() <= MAX_CHARS && step.split_whitespace().count() >= 2;
    usable.then(|| step.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offer_at_the_end_becomes_the_step() {
        for (answer, step) in [
            ("All done.\n\nWant me to commit this and tag `v1.1.0`?", "commit this and tag `v1.1.0`"),
            ("Tests pass. Should I tag v1.1.0 and push both to origin?", "tag v1.1.0 and push both to origin"),
            ("Would you like me to add tests for the parser?", "add tests for the parser"),
            ("Do you want me to start on those?", "start on those"),
            ("Let me know if you'd like me to update the README as well.", "update the README"),
            ("If you want, I can also wire it into the TUI.", "wire it into the TUI"),
            ("**Want me to fix the two failing tests now?**", "fix the two failing tests"),
            ("Shall I open a pull request, or keep it local?", "open a pull request"),
        ] {
            assert_eq!(next_prompt(answer).as_deref(), Some(step), "{answer}");
        }
    }

    #[test]
    fn the_last_offer_wins_and_answers_without_one_suggest_nothing() {
        let two = "Want me to rename it? Also, want me to push the branch?";
        assert_eq!(next_prompt(two).as_deref(), Some("push the branch"));
        for none in [
            "",
            "Done. The build passes.",
            "Why does this fail?",
            "Want me to?",
            "Should I?",
            "Want me to run \x1b]52;c;QUFB\x07 this?",
            "I can also see that the parser fails on empty input.",
            "I'm happy to report the build passes everywhere.",
        ] {
            assert_eq!(next_prompt(none), None, "{none:?}");
        }
    }

    #[test]
    fn an_offer_far_from_the_end_or_too_long_is_not_a_prompt() {
        let early = format!("Want me to refactor the cache? {}", "More detail follows. ".repeat(40));
        assert_eq!(next_prompt(&early), None);
        let long = format!("Want me to {}?", "do one more careful thing ".repeat(5));
        assert_eq!(next_prompt(&long), None);
    }

    #[test]
    fn non_ascii_text_is_handled_on_character_boundaries() {
        assert_eq!(next_prompt("日本語のテスト。 Want me to translate the docs?").as_deref(), Some("translate the docs"));
        assert_eq!(next_prompt(&"é".repeat(400)), None);
    }
}
