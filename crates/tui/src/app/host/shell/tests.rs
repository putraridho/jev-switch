use super::*;

#[test]
fn a_shell_command_runs_in_the_project_and_reports_its_exit() {
    let dir = std::env::temp_dir();
    let mut seen = Vec::new();
    let ok = run_shell("echo out; echo err >&2", &dir, &mut |l| seen.push(l.to_string()));
    assert_eq!((ok.output.as_str(), ok.code), ("out\nerr", Some(0)));
    assert_eq!(seen, ["out", "err"], "each line is passed on as it comes");
    // A job left in the background holds the pipe open; the command still returns.
    let started = std::time::Instant::now();
    let bg = run_shell("sleep 30 & echo started", &dir, &mut |_| {});
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "waited for a background job: {:?}", started.elapsed());
    assert_eq!((bg.output.as_str(), bg.code), ("started", Some(0)));
    let bad = run_shell("exit 3", &dir, &mut |_| {});
    assert_eq!(bad.code, Some(3));
    assert!(shell_note(&bad).contains("it exited 3"));
    let note = shell_note(&ok);
    assert!(note.starts_with("I ran `echo out; echo err >&2` in the shell; it exited 0. Its output:\n```\nout\nerr\n```"), "{note}");
}

#[test]
fn long_output_keeps_its_end() {
    let many: String = (0..SHELL_KEEP_LINES + 5).map(|i| format!("line {i}\n")).collect();
    let (kept, cut) = keep_tail(&many);
    assert!(cut && kept.starts_with("line 5\n") && kept.ends_with(&format!("line {}", SHELL_KEEP_LINES + 4)));
    let wide = "é".repeat(SHELL_KEEP_BYTES);
    let (kept, cut) = keep_tail(&wide);
    assert!(cut && kept.len() <= SHELL_KEEP_BYTES);
    assert_eq!(keep_tail("short"), ("short".to_string(), false));
    let (output, truncated) = keep_tail(&many);
    let note = shell_note(&ShellRun { command: "x".into(), output, truncated, code: Some(0), failed: None });
    assert!(note.contains("(only the end is shown)"));
}

#[test]
fn bounded_tail_matches_full_output_policy() {
    let cases = [
        "".to_string(),
        "short\r\n\nlast\r".to_string(),
        "double\r\r\nnext".to_string(),
        (0..205).map(|index| format!("line {index}\n")).collect(),
        format!("{}\n{}\n", "é".repeat(SHELL_KEEP_BYTES), "x".repeat(400)),
        format!("{}\n{}", "a".repeat(SHELL_KEEP_BYTES - 1), "é"),
        "\n".repeat(205),
        format!("{}\n\n", "a".repeat(SHELL_KEEP_BYTES)),
        format!("{}\nend\n", "a".repeat(30_000)),
        format!("{}\r\nend\n", "a".repeat(SHELL_KEEP_BYTES - 1)),
        format!("{}\r\n", "a".repeat(SHELL_KEEP_BYTES)),
        format!("{}\nend", "é".repeat(SHELL_KEEP_BYTES)),
    ];
    for text in cases {
        let mut tail = ShellTail::default();
        let (mut reader, mut pending) = (text.as_bytes(), Vec::new());
        while let Some(piece) = next_piece(&mut reader, &mut pending, SHELL_KEEP_BYTES) {
            tail.push(std::str::from_utf8(&piece).unwrap());
            assert!(tail.lines.len() <= SHELL_KEEP_LINES);
            assert!(tail.bytes <= SHELL_KEEP_BYTES);
            assert_eq!(tail.bytes, tail.lines.iter().map(String::len).sum::<usize>() + tail.lines.len().saturating_sub(1));
        }
        let expected = keep_tail(&text);
        let cut = tail.cut;
        let output = tail.into_text();
        assert_eq!((&output, cut), (&expected.0, expected.1));
        let actual = shell_note(&ShellRun { command: "x".into(), output, truncated: cut, code: Some(0), failed: None });
        let marker = if expected.1 { " (only the end is shown)" } else { "" };
        assert_eq!(actual, format!("I ran `x` in the shell; it exited 0. Its output{marker}:\n```\n{}\n```", expected.0.trim_end()));
    }
}

#[test]
fn bounded_tail_never_exceeds_its_caps() {
    let mut tail = ShellTail::default();
    for index in 0..100_000 {
        tail.push(&format!("{index}: {}\n", "é".repeat(100)));
        assert!(tail.lines.len() <= SHELL_KEEP_LINES);
        assert!(tail.bytes <= SHELL_KEEP_BYTES);
        assert_eq!(tail.bytes, tail.lines.iter().map(String::len).sum::<usize>() + tail.lines.len().saturating_sub(1));
    }
}

#[test]
fn pieces_are_lines_or_bounded_cuts_on_character_starts() {
    let pieces = |text: &[u8], cap: usize| {
        let (mut reader, mut pending, mut out) = (text, Vec::new(), Vec::new());
        while let Some(piece) = next_piece(&mut reader, &mut pending, cap) {
            assert!(piece.len() <= cap.max(piece.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1)));
            out.push(String::from_utf8(piece).unwrap());
        }
        out
    };
    assert_eq!(pieces(b"a\nbc\nd", 10), ["a\n", "bc\n", "d"]);
    assert_eq!(pieces(b"abcdefgh\n", 3), ["abc", "def", "gh\n"]);
    assert_eq!(pieces(b"ab\r\n", 3), ["ab", "\r\n"]);
    // `é` is two bytes: a cut never splits it.
    assert_eq!(pieces("aéé".as_bytes(), 2), ["a", "é", "é"]);
    assert!(pieces(b"", 4).is_empty());
}

#[test]
fn output_without_line_breaks_stays_bounded() {
    let run = run_shell("yes x | tr -d '\\n' | head -c 3000000", Path::new("."), &mut |line| assert!(line.len() <= SHELL_KEEP_BYTES));
    assert_eq!(run.code, Some(0));
    assert!(run.truncated && run.output.len() <= SHELL_KEEP_BYTES, "{}", run.output.len());
}

#[test]
fn shell_retains_long_lines_without_inserting_newlines() {
    let run = run_shell("printf '%030000d\\nend\\n' 0 | tr 0 a", Path::new("."), &mut |_| {});
    let full = format!("{}\nend\n", "a".repeat(30_000));
    assert_eq!(run.code, Some(0));
    assert_eq!((run.output.as_str(), run.truncated), (keep_tail(&full).0.as_str(), true));
    assert!(run.output.ends_with("\nend"));
    assert!(!run.output.strip_suffix("\nend").unwrap().contains('\n'));
}

#[test]
fn shell_does_not_add_an_empty_line_after_a_cap_sized_line() {
    let mut seen = Vec::new();
    let run = run_shell("printf '%020000d\\nend\\n' 0 | tr 0 a", Path::new("."), &mut |line| seen.push(line.to_string()));
    let full = format!("{}\nend\n", "a".repeat(SHELL_KEEP_BYTES));
    assert_eq!(run.code, Some(0));
    assert_eq!((run.output.as_str(), run.truncated), (keep_tail(&full).0.as_str(), true));
    assert!(!run.output.contains("\n\n"));
    // The live view shows no empty row the model's tail does not have either.
    assert!(!seen.iter().any(String::is_empty), "{:?}", seen.iter().map(String::len).collect::<Vec<_>>());
    assert_eq!(seen.last().map(String::as_str), Some("end"));
}

#[test]
fn shell_streams_every_line_while_retaining_only_the_tail() {
    let command = "i=0; while [ \"$i\" -lt 1000 ]; do printf 'line %s\\n' \"$i\"; i=$((i + 1)); done";
    let mut seen = Vec::new();
    let run = run_shell(command, Path::new("."), &mut |line| seen.push(line.to_string()));
    assert_eq!(run.code, Some(0));
    assert_eq!(seen.len(), 1000);
    let full = seen.join("\n");
    let (expected, truncated) = keep_tail(&full);
    assert_eq!(run.output, expected);
    assert_eq!(run.truncated, truncated);
    assert!(shell_note(&run).contains("(only the end is shown)"));
}
