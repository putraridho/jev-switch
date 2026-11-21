use super::*;

    const EVIL: &str = "a\x1b]52;c;QUFB\x07b\x1b]8;;http://x\x1b\\c\x1b[2Jd\u{9b}2Je\rf";

    fn assert_safe(out: &str) {
        let plain = strip_ansi(out);
        for bad in ['\x07', '\r', '\u{9b}', '\u{9d}', '\x08'] {
            assert!(!out.contains(bad), "{bad:?} in {out:?}");
        }
        assert!(!out.contains("]52") && !out.contains("]8;") && !out.contains("[2J"), "{out:?}");
        assert!(plain.contains("abcd"), "{plain:?}");
    }

    #[test]
    fn untrusted_text_in_output_edits_and_reasoning_is_neutralised() {
        let out = tool_output_fold("Bash", false, &format!("first\n{EVIL}"), ON, 80).unwrap_or_default();
        assert_safe(&out);
        assert!(out.contains("\x1b[2m"), "our colour stays: {out:?}");
        let edit = edit_preview(&[EVIL.to_string()], &[EVIL.to_string()], ON, 80);
        assert_safe(&edit);
        assert!(edit.contains("\x1b[31m") && edit.contains("\x1b[32m"));
        assert_safe(&thinking_block(EVIL, ON, 80));
        assert!(thinking_block(EVIL, ON, 80).contains("\x1b[3m"));
        assert_safe(&narration(EVIL, ON, 80));
    }
