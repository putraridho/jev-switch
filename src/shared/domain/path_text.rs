//! Paths named in a prompt, and glob matching on them. Pure text work: no policy, no IO.

use std::path::Path;

/// The raw and unwrapped words of a prompt, relative to `root`.
pub fn mentioned_paths(root: &Path, prompt: &str) -> Vec<String> {
    let wrapper = |c: char| matches!(c, '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | '?' | '!' | ':');
    prompt
        .split_whitespace()
        .flat_map(|raw| {
            // Quotes and locations nest either way (`` `a.env`:3 ``, `"a.env:3"`), so both are
            // taken off until nothing changes.
            let mut unwrapped = raw.trim_start_matches(|c| wrapper(c) || c == '@');
            loop {
                let next = strip_location_suffixes(unwrapped.trim_end_matches(|c| wrapper(c) || c == '.'));
                if next == unwrapped {
                    break;
                }
                unwrapped = next;
            }
            [raw, unwrapped]
        })
        .filter(|w| !w.is_empty())
        .map(|w| relative(root, w))
        .collect()
}

fn strip_location_suffixes(mut path: &str) -> &str {
    fn digits(text: &str) -> bool {
        !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
    }

    loop {
        if let Some((base, location)) = path.rsplit_once(':') {
            let is_location = match location.split_once('-') {
                Some((start, end)) => digits(start) && digits(end),
                None => digits(location),
            };
            if is_location {
                path = base;
                continue;
            }
        }
        if let Some((base, location)) = path.rsplit_once("#L") {
            let is_location = match location.split_once("-L") {
                Some((start, end)) => digits(start) && digits(end),
                None => digits(location),
            };
            if is_location {
                path = base;
                continue;
            }
        }
        return path;
    }
}

/// `path` as the policy sees it: `~` expanded, `.` and `..` resolved (so `a/../secrets/x`
/// is `secrets/x`), and the project root taken off the front.
pub fn relative(root: &Path, path: &str) -> String {
    let home = std::env::var("HOME").ok().filter(|h| h.starts_with('/'));
    let expanded = match (path, &home) {
        ("~", Some(h)) => h.clone(),
        (p, Some(h)) if p.starts_with("~/") => format!("{}/{}", h.trim_end_matches('/'), &p[2..]),
        _ => path.to_string(),
    };
    let absolute = expanded.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in expanded.split('/') {
        match seg {
            "" | "." => {}
            ".." => match parts.last() {
                Some(last) if *last != ".." => {
                    parts.pop();
                }
                _ if absolute => {}
                _ => parts.push(".."),
            },
            s => parts.push(s),
        }
    }
    let normal = parts.join("/");
    let root = root.to_string_lossy();
    let root = root.trim_end_matches('/');
    if absolute && !root.is_empty() {
        let root_normal = root.trim_start_matches('/');
        if normal.eq_ignore_ascii_case(root_normal) {
            return String::new();
        }
        if let Some(prefix) = normal.get(..root_normal.len()) {
            if prefix.eq_ignore_ascii_case(root_normal) {
                if let Some(rest) = normal[root_normal.len()..].strip_prefix('/') {
                    return rest.to_string();
                }
            }
        }
    }
    normal
}

/// `*` and `?` inside a path segment, `**` as a whole segment for any number of segments.
/// A trailing `/` means everything below. A pattern with no `/` is tried on every segment
/// of the path, so `.env` and `*.pem` match at any depth. ASCII case is ignored.
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern = if pattern.ends_with('/') { format!("{pattern}**") } else { pattern.to_string() };
    let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if pattern.contains('/') {
        let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
        return segments_match(&pat, &path_segments);
    }
    path_segments.iter().any(|s| segment_matches(&pattern, s))
}

fn segments_match(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| segments_match(rest, &path[skip..])),
        Some((first, rest)) => path.split_first().is_some_and(|(p, tail)| segment_matches(first, p) && segments_match(rest, tail)),
    }
}

fn segment_matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    fn go(p: &[char], t: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('*', rest)) => (0..=t.len()).any(|n| go(rest, &t[n..])),
            Some(('?', rest)) => t.split_first().is_some_and(|(_, tail)| go(rest, tail)),
            Some((c, rest)) => t.split_first().is_some_and(|(x, tail)| x.eq_ignore_ascii_case(c) && go(rest, tail)),
        }
    }
    go(&p, &t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/work/app";

    #[test]
    fn globs() {
        let yes = [("secrets/**", "secrets/a/b.txt"), ("secrets/", "secrets/x"), ("*.pem", "deploy/keys/a.pem"), (".env", "app/.env"), ("src/*.rs", "src/a.rs"), ("**/prod/**", "a/b/prod/c"), ("a?c", "abc")];
        for (pat, path) in yes {
            assert!(glob_matches(pat, path), "{pat} should match {path}");
        }
        let no = [("secrets/**", "src/secrets.rs"), ("src/*.rs", "src/a/b.rs"), ("*.pem", "a.pem.txt"), ("secrets/**", "a/secrets/x"), (".env", ".environment")];
        for (pat, path) in no {
            assert!(!glob_matches(pat, path), "{pat} should not match {path}");
        }
    }

    #[test]
    fn non_location_suffixes_are_preserved() {
        for prompt in ["x.pem:12-x", "x.pem:12-", "x.pem#L", "x.pem#L1-L", "x.pem#L1-extra", "C:\\work\\x.pem"] {
            assert_eq!(mentioned_paths(Path::new(ROOT), prompt), vec![prompt, prompt]);
        }
        assert_eq!(mentioned_paths(Path::new(ROOT), "host:8080"), vec!["host:8080", "host"]);
    }
}
