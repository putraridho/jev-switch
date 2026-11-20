//! Splitting a command line into simple commands and words, refusing what hides what runs.

use crate::approval::domain::command_class::is_shell_syntax;
use crate::approval::domain::refusal::Refusal;
use crate::approval::domain::visible_text::is_invisible;

/// One word of a simple command, with its quotes removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Word {
    pub(super) text: String,
    /// Some part of it was in quotes.
    pub(super) quoted: bool,
}

/// How the previous simple command of a chain ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Joint {
    /// `;` or a line break: nothing has to follow.
    Sequence,
    /// `&&`, `||` or `|`: a command has to follow.
    Operator,
}

/// A `timeout 900` or `time` in front: it only limits or measures the command after it, so
/// the rule is that command's. Anything with flags (`timeout -s KILL`) is left as it is.
pub(super) fn without_time_wrapper(words: &[String]) -> &[String] {
    let is_duration = |w: &str| {
        let digits = w.trim_end_matches(['s', 'm', 'h', 'd']);
        !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit() || c == '.') && digits.chars().next().is_some_and(|c| c.is_ascii_digit())
    };
    match words {
        [first, duration, rest @ ..] if first == "timeout" && is_duration(duration) && !rest.is_empty() => rest,
        [first, rest @ ..] if first == "time" && !rest.is_empty() => rest,
        _ => words,
    }
}

/// Split a chain into simple commands of literal words. Accepted between commands: `&&`, `||`,
/// `|`, `;` and line breaks; in a command: words, '…' (anything but control characters, a line
/// break is text there) and "…" (without `$`, backticks or an escape inside; a backslash that
/// escapes nothing is a backslash), and the redirections that write no file (`2>&1`, `>&2`, `>/dev/null`,
/// `2>/dev/null`, `&>/dev/null`). Everything else (`$`, backticks, `\`, `<`, heredocs,
/// subshells, braces, `!`, a `#` comment, a background `&`, a redirection to a file) is refused:
/// the rules would not describe what runs.
pub(super) fn split_chain(command: &str) -> Result<Vec<Vec<Word>>, Refusal> {
    const BAD: Refusal = Refusal::ShellSyntax;
    let chars: Vec<char> = command.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let ends_target = |c: char| c == ' ' || c == '\t' || matches!(c, ';' | '&' | '|' | '<' | '>' | '(' | ')' | '\n');
    let mut segments: Vec<Vec<Word>> = Vec::new();
    let mut words: Vec<Word> = Vec::new();
    let mut word = Word { text: String::new(), quoted: false };
    let mut in_word = false;
    let mut last = Joint::Sequence;

    fn end_word(word: &mut Word, in_word: &mut bool, words: &mut Vec<Word>) {
        if *in_word {
            words.push(std::mem::replace(word, Word { text: String::new(), quoted: false }));
            *in_word = false;
        }
    }
    fn end_segment(words: &mut Vec<Word>, segments: &mut Vec<Vec<Word>>, joint: Joint, last: &mut Joint) -> Result<(), Refusal> {
        if words.is_empty() {
            // `; ;`, a chain that starts with an operator, or `a && ;`.
            if joint == Joint::Operator || *last == Joint::Operator || segments.is_empty() {
                return Err(BAD);
            }
        } else {
            segments.push(std::mem::take(words));
        }
        *last = joint;
        Ok(())
    }

    let mut i = 0;
    while let Some(c) = at(i) {
        match c {
            ' ' | '\t' => end_word(&mut word, &mut in_word, &mut words),
            '\'' => {
                in_word = true;
                word.quoted = true;
                i += 1;
                loop {
                    match at(i) {
                        None => return Err(BAD),
                        Some('\'') => break,
                        Some(q) if (q.is_control() && !matches!(q, '\t' | '\n')) || is_invisible(q) => return Err(BAD),
                        Some(q) => word.text.push(q),
                    }
                    i += 1;
                }
            }
            '"' => {
                in_word = true;
                word.quoted = true;
                i += 1;
                loop {
                    match at(i) {
                        None => return Err(BAD),
                        Some('"') => break,
                        // In double quotes a backslash is itself unless it escapes one of these
                        // (`grep "a\|b"` is a backslash and a bar); an escape is not followed.
                        Some('\\') if at(i + 1).is_some_and(|n| !(matches!(n, '$' | '`' | '"' | '\\') || n.is_control() || is_invisible(n))) => word.text.push('\\'),
                        Some(q) if matches!(q, '$' | '`' | '\\') || (q.is_control() && !matches!(q, '\t' | '\n')) || is_invisible(q) => return Err(BAD),
                        Some(q) => word.text.push(q),
                    }
                    i += 1;
                }
            }
            ';' | '\n' => {
                end_word(&mut word, &mut in_word, &mut words);
                end_segment(&mut words, &mut segments, Joint::Sequence, &mut last)?;
            }
            '&' if at(i + 1) == Some('&') => {
                end_word(&mut word, &mut in_word, &mut words);
                end_segment(&mut words, &mut segments, Joint::Operator, &mut last)?;
                i += 1;
            }
            '|' if at(i + 1) == Some('&') => return Err(BAD),
            '|' => {
                end_word(&mut word, &mut in_word, &mut words);
                end_segment(&mut words, &mut segments, Joint::Operator, &mut last)?;
                if at(i + 1) == Some('|') {
                    i += 1;
                }
            }
            '>' | '&' => {
                // `&>` needs `>` next; a lone `&` runs in the background.
                if c == '&' {
                    if at(i + 1) != Some('>') {
                        return Err(BAD);
                    }
                    i += 1;
                }
                // A file-descriptor number right before `>` (`2>`) belongs to the redirection.
                if in_word {
                    if c == '>' && !word.quoted && word.text.chars().all(|d| d.is_ascii_digit()) {
                        word.text.clear();
                        in_word = false;
                    } else {
                        return Err(BAD);
                    }
                }
                i += 1;
                if at(i) == Some('>') {
                    i += 1;
                }
                if at(i) == Some('&') {
                    // `>&2`: duplicate a descriptor, no file.
                    i += 1;
                    let start = i;
                    while at(i).is_some_and(|d| d.is_ascii_digit()) {
                        i += 1;
                    }
                    if i == start || at(i).is_some_and(|n| !ends_target(n)) {
                        return Err(BAD);
                    }
                } else {
                    while matches!(at(i), Some(' ' | '\t')) {
                        i += 1;
                    }
                    let mut target = String::new();
                    while let Some(t) = at(i).filter(|t| !ends_target(*t)) {
                        target.push(t);
                        i += 1;
                    }
                    if target != "/dev/null" {
                        return Err(BAD);
                    }
                }
                continue;
            }
            '#' if !in_word => return Err(BAD),
            c if is_shell_syntax(c) => return Err(BAD),
            c => {
                word.text.push(c);
                in_word = true;
            }
        }
        i += 1;
    }
    end_word(&mut word, &mut in_word, &mut words);
    if words.is_empty() {
        if last == Joint::Operator || segments.is_empty() {
            return Err(BAD);
        }
    } else {
        segments.push(words);
    }
    Ok(segments)
}
