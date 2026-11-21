//! The request as Jev reads it. A long paste (a page of HTML, a log, a diff) drowns the ask in
//! noise, and the ask usually sits at the start or the end; judging those two ends reads what
//! the person wants. Only the judgment sees the condensed text: the model gets the whole prompt.

use std::borrow::Cow;

/// Characters kept from the start and from the end of a long request.
pub const JUDGED_HEAD: usize = 1500;
pub const JUDGED_TAIL: usize = 1500;

/// `request` as is when it is short, else its start and its end with a note between.
pub fn judged_text(request: &str) -> Cow<'_, str> {
    let total = request.chars().count();
    if total <= JUDGED_HEAD + JUDGED_TAIL + 200 {
        return Cow::Borrowed(request);
    }
    let head: String = request.chars().take(JUDGED_HEAD).collect();
    let tail: String = request.chars().skip(total - JUDGED_TAIL).collect();
    let cut = total - JUDGED_HEAD - JUDGED_TAIL;
    Cow::Owned(format!("{head}\n[… {cut} characters of pasted text left out …]\n{tail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_request_is_judged_whole() {
        assert_eq!(judged_text("fix the login bug"), "fix the login bug");
    }

    #[test]
    fn a_long_paste_keeps_its_start_and_its_end() {
        let request = format!("look at this\n{}\nplease validate the 6 comments", "<div class=\"x\"></div>".repeat(2000));
        let judged = judged_text(&request);
        assert!(judged.starts_with("look at this") && judged.ends_with("please validate the 6 comments"));
        assert!(judged.contains("characters of pasted text left out"));
        assert!(judged.chars().count() < 3300);
    }

    #[test]
    fn multibyte_text_is_cut_on_characters() {
        let long = "é".repeat(10_000);
        let judged = judged_text(&long);
        assert!(judged.contains("left out"));
    }
}
