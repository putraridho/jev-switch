use super::super::approval::*;

#[test]
fn approval_keys_answer_only_when_they_mean_it() {
    use AnswerKey::*;
    for (byte, offered, want) in [
        (b'y', true, Some(Yes)),
        (b'Y', false, Some(Yes)),
        (b'n', true, Some(No)),
        (0x04, true, Some(No)),
        (b'a', true, Some(Always)),
        (b'A', true, Some(Always)),
        (b'a', false, None),
        (0x03, false, Some(Interrupt)),
        (b'\r', true, None),
        (b'\n', true, None),
        (b' ', true, None),
        (b'q', true, None),
        (b'[', true, None),
    ] {
        assert_eq!(answer_for_byte(byte, offered), want, "{byte:#x} offered={offered}");
    }
}

/// With no terminal on stdin (as under `cargo test`) the question answers no and returns.
#[cfg(unix)]
#[test]
fn reading_a_key_without_a_terminal_never_waits() {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        assert_eq!(read_answer(true, &std::sync::atomic::AtomicBool::new(false)), Some(AnswerKey::No));
    }
}
