//! Recognising a used-up plan or usage limit in what a CLI says. Owns the one phrase list
//! the chat client and every backend ask. Must not depend on any other module.

/// How far after "limit" the word "resets" may come and still be the limit's reset time.
const RESETS_WITHIN: usize = 30;

/// Most words that may come before "limit" in a limit notice ("you've hit your Opus
/// weekly limit" has five); longer lead-ins are ordinary sentences.
const MAX_WORDS_BEFORE_LIMIT: usize = 6;

/// Wording of a used-up plan or usage limit, as Claude Code and the other CLIs say it
/// ("You've hit your session limit", "5-hour limit reached ∙ resets 3pm", "Opus weekly
/// limit reached", "Claude AI usage limit reached|<epoch>", "Approaching usage limit").
/// `lower` must be lowercased with typographic apostrophes already replaced. This is
/// the ONE phrase source: the shared error classifier (`backend::classify_common`) and
/// [`is_limit_message`] both ask it. Phrases only, never a bare "limit" or "resets":
/// "limit the query to 10 rows" and "the resets handler" are ordinary text.
pub fn is_limit_phrase(lower: &str) -> bool {
    let after = |needle: &str| lower.split_once(needle).map(|(_, rest)| rest);
    lower.contains("usage limit")
        || lower.contains("limit reached")
        || (lower.contains("hit your") && lower.contains("limit"))
        || after("reached your").is_some_and(|rest| rest.contains("limit"))
        // "<x> limit ∙ resets <when>": the reset comes right after the limit, in the same breath.
        || after("limit").is_some_and(|rest| {
            rest.find("resets").is_some_and(|i| i <= RESETS_WITHIN && !rest[..i].contains(['.', ';']))
        })
}

/// Claude Code reports a used-up plan limit as the answer text, with no error flag.
/// Shown as an answer it reads like the model replying; it must be treated as a
/// usage-limit error. Only the opening sentence is checked, and "limit" must come
/// early in it, so a real answer that merely mentions limits isn't caught.
pub fn is_limit_message(text: &str) -> bool {
    let head: String = text.trim_start().chars().take(120).collect::<String>().to_lowercase();
    let head = head.replace('\u{2019}', "'");
    let sentence = head.split(['.', ';', '\n', '!', '?']).next().unwrap_or("");
    let lead_words = sentence.split_once("limit").map_or(usize::MAX, |(lead, _)| lead.split_whitespace().count());
    lead_words <= MAX_WORDS_BEFORE_LIMIT && is_limit_phrase(sentence)
}

#[cfg(test)]
mod tests {
    use super::{is_limit_message, is_limit_phrase};

    #[test]
    fn real_claude_limit_notices_are_recognised() {
        for text in [
            "5-hour limit reached \u{2219} resets 3pm",
            "Weekly limit reached \u{2219} resets Nov 24",
            "Opus weekly limit reached",
            "You've hit your session limit \u{b7} resets 5pm (Asia/Jakarta)",
            "Claude AI usage limit reached|1794853451",
            "Approaching usage limit",
            "Limit reached",
            "You have reached your weekly limit",
        ] {
            assert!(is_limit_message(text), "{text:?}");
        }
    }

    #[test]
    fn ordinary_text_mentioning_limits_is_not_a_limit_phrase() {
        for text in [
            "the resets handler",
            "rate limit middleware",
            "limit the query to 10 rows",
            "quota calculation function",
            "the limit is 10; the timer resets later",
            "when the timer resets the counter goes to zero",
        ] {
            assert!(!is_limit_phrase(text), "{text:?}");
            assert!(!is_limit_message(text), "{text:?}");
        }
        assert!(!is_limit_message("I read the whole module and the limit reached in the loop is fine."));
    }

    #[test]
    fn plan_limit_text_is_recognised_but_answers_about_limits_are_not() {
        assert!(is_limit_message("You've hit your session limit · resets 5pm (Asia/Jakarta)"));
        assert!(is_limit_message("You\u{2019}ve hit your weekly limit"));
        assert!(is_limit_message("  Claude AI usage limit reached|1794853451"));
        assert!(!is_limit_message("The rate limit is 50 requests per minute; you've hit your limit when it returns 429."));
        assert!(!is_limit_message("Fixed the off-by-one in the pagination limit."));
        assert!(!is_limit_message(""));
    }
}
