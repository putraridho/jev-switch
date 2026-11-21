//! Owns the rules for when and with what output goes through a pager. Must not start one.



/// The pager to start for `$PAGER`: its program and arguments, or `less -R` (which keeps
/// colours) when it is unset or blank.
pub fn pager_command(pager_env: Option<&str>) -> (String, Vec<String>) {
    let mut words = pager_env.unwrap_or("").split_whitespace().map(String::from);
    match words.next() {
        Some(program) => (program, words.collect()),
        None => ("less".to_string(), vec!["-R".to_string()]),
    }
}

/// Whether `text` goes through a pager: only on a terminal, and only when it would scroll
/// off the screen. Never when there is no terminal, so a pipe is never blocked.
pub fn should_page(text: &str, rows: Option<usize>, stdout_is_tty: bool) -> bool {
    stdout_is_tty && text.lines().count() > rows.unwrap_or(24).saturating_sub(2)
}
