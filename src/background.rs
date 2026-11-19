//! Work a model leaves running when its turn ends. A turn ends when the model stops: nothing
//! wakes it when a background command finishes, so "I'll update you when the tests finish"
//! is a promise no model can keep. Every provider is told so ([`NOTE`]), and a turn that still
//! did it gets a notice saying to ask again ([`notice`]).

/// What every model is told, as a system note or after the prompt: wait for commands, and do
/// not promise to come back.
pub const NOTE: &str = "You cannot speak again until the user sends a message: nothing wakes you when a command finishes. \
Run commands in the foreground and wait for them to finish before you answer; do not start them in the background (`&`, nohup, async or background task options) unless the user asks for that, \
and never end your turn saying you will report back or update the user later.";

/// The start of every notice, so a notice raised by a backend is not repeated by the turn.
pub const NOTICE_PREFIX: &str = "\u{26a0} still running";

/// `prompt` with [`NOTE`] after it, for CLIs that take no system note of their own.
pub fn with_note(prompt: &str) -> String {
    format!("{prompt}\n\n(jev-code: {NOTE})")
}

fn is_shell(tool: &str) -> bool {
    matches!(tool.to_ascii_lowercase().as_str(), "bash" | "shell" | "run_shell_command" | "run_command" | "exec" | "execute" | "terminal")
}

/// Does this shell command leave something running after it returns? A trailing `&` (not
/// `&&`), `nohup`, `setsid` or `disown`.
pub fn backgrounds(command: &str) -> bool {
    let c = command.trim();
    let words: Vec<&str> = c.split(|ch: char| ch.is_whitespace() || ch == ';' || ch == '(' || ch == ')').filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| matches!(*w, "nohup" | "setsid" | "disown")) {
        return true;
    }
    // `cmd &`, `cmd & echo started`, `cmd &>log &`: an `&` that is neither `&&` nor a redirect.
    let bytes = c.as_bytes();
    (0..bytes.len()).any(|i| {
        bytes[i] == b'&'
            && bytes.get(i + 1).is_none_or(|n| *n != b'&' && *n != b'>')
            && (i == 0 || (bytes[i - 1] != b'&' && bytes[i - 1] != b'>' && bytes[i - 1] != b'<' && bytes[i - 1] != b'|'))
    })
}

/// How many of the turn's tool calls (`Bash npm test &`, one-line summaries) started something
/// in the background.
pub fn spawned(tools: &[String]) -> usize {
    tools
        .iter()
        .filter(|t| {
            let (tool, rest) = t.trim().split_once(' ').unwrap_or((t.trim(), ""));
            is_shell(tool) && backgrounds(rest)
        })
        .count()
}

/// Whether a tool's output says it left processes running (Gemini CLI's shell tool reports
/// `Background PIDs: 1234`).
pub fn result_reports_background(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim();
        l.strip_prefix("Background PIDs:").is_some_and(|rest| rest.chars().any(|c| c.is_ascii_digit()))
    })
}

/// Whether the answer promises to come back later on its own ("I'll update you when the
/// tests finish").
pub fn promises_update(answer: &str) -> bool {
    let a = answer.to_lowercase().replace('\u{2019}', "'");
    let promise = ["i will update you", "i'll update you", "i will let you know", "i'll let you know", "i will report back", "i'll report back", "i will notify you", "i'll notify you", "will update you as soon as", "i'll get back to you", "i will get back to you", "i'll check back", "i will check back"];
    promise.iter().any(|p| a.contains(p))
}

/// The notice for a turn that left work running or promised to come back, if it did.
pub fn notice(running: usize, promised: bool) -> Option<String> {
    match (running, promised) {
        (0, false) => None,
        (0, true) => Some(format!("{NOTICE_PREFIX}? the model said it would report back, but it cannot until you send a message: ask it to check when you want the result")),
        (1, _) => Some(format!("{NOTICE_PREFIX}: 1 command was left running in the background; the model will not report on it by itself, so ask it to check")),
        (n, _) => Some(format!("{NOTICE_PREFIX}: {n} commands were left running in the background; the model will not report on them by itself, so ask it to check")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_commands_are_told_from_chained_and_redirected_ones() {
        for bg in ["npm test &", "npm run dev & echo started", "nohup ./server", "setsid sleep 9", "sleep 5 & disown", "cargo test &>log &", "(make &)"] {
            assert!(backgrounds(bg), "{bg}");
        }
        for fg in ["cargo test && echo ok", "make 2>&1 | tail", "ls &>/dev/null", "a || b", "echo '&&'", "cat <&3", "grep -n x f"] {
            assert!(!backgrounds(fg), "{fg}");
        }
    }

    #[test]
    fn only_shell_calls_count_and_tool_output_can_say_so() {
        let tools = vec!["Bash npm test &".to_string(), "Read src/a&b.rs".to_string(), "run_command nohup make".to_string(), "Bash ls".to_string()];
        assert_eq!(spawned(&tools), 2);
        assert!(result_reports_background("Output: ok\nBackground PIDs: 4242\n"));
        assert!(!result_reports_background("Background PIDs: (none)"));
    }

    #[test]
    fn promises_to_come_back_are_noticed_in_any_apostrophe() {
        assert!(promises_update("I am running the tests. I will update you as soon as the run finishes."));
        assert!(promises_update("Started the build; I\u{2019}ll let you know when it's done."));
        assert!(!promises_update("The tests pass. Let me know if you want more."));
    }

    #[test]
    fn the_notice_says_what_was_left_and_to_ask() {
        assert_eq!(notice(0, false), None);
        assert!(notice(0, true).unwrap().starts_with(NOTICE_PREFIX));
        assert!(notice(2, true).unwrap().contains("2 commands"));
        assert!(with_note("fix it").starts_with("fix it\n\n(jev-code: You cannot speak again"));
    }
}
