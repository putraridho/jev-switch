// A file belongs to the bounded context named by its top-level folder under `src/`
// (`src/approval/...` is `approval`). The map of which context may import which is EDGES.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const EDGES: &[(&str, &[&str])] = &[
    ("shared", &[]), ("catalog", &["shared"]), ("approval", &["shared"]),
    ("changes", &["shared"]), ("judgment", &["catalog", "shared"]),
    ("backend", &["approval", "shared"]), ("routing", &["judgment", "catalog", "shared"]),
    ("providers", &["backend", "catalog", "shared"]), ("session", &["backend", "shared"]),
    ("orchestration", &["backend", "providers", "catalog", "judgment", "shared"]),
    ("doctor", &["providers", "judgment", "shared"]),
    ("chat", &["judgment", "catalog", "shared"]),
    ("turn", &["shared", "catalog", "approval", "changes", "judgment", "backend", "routing", "providers", "session", "orchestration"]),
    ("gateway", &["orchestration", "routing", "judgment", "catalog", "shared"]),
    ("tui", &["turn", "doctor", "providers", "backend", "approval", "changes", "session", "routing", "judgment", "catalog", "orchestration", "shared"]),
    ("calibration", &["gateway", "routing", "judgment", "catalog", "shared"]),
    ("embed", &["turn", "providers", "backend", "approval", "routing", "changes", "session", "shared"]),
    ("cli", &["tui", "gateway", "orchestration", "backend", "chat", "calibration", "shared"]),
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Token { text: String, line: usize }

fn lex(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let (mut pos, mut line) = (0, 1);
    while pos < bytes.len() {
        let start = pos;
        let start_line = line;
        if bytes[pos].is_ascii_whitespace() { pos += 1; }
        else if bytes[pos..].starts_with(b"//") {
            pos += 2;
            while pos < bytes.len() && bytes[pos] != b'\n' { pos += 1; }
        } else if bytes[pos..].starts_with(b"/*") {
            pos += 2;
            let mut depth = 1;
            while pos < bytes.len() && depth > 0 {
                if bytes[pos..].starts_with(b"/*") { depth += 1; pos += 2; }
                else if bytes[pos..].starts_with(b"*/") { depth -= 1; pos += 2; }
                else { pos += 1; }
            }
        } else {
            let raw_start = if bytes[pos..].starts_with(b"br") { Some(pos + 2) }
                else if bytes[pos] == b'r' { Some(pos + 1) } else { None };
            let raw = raw_start.and_then(|mut end| {
                while end < bytes.len() && bytes[end] == b'#' { end += 1; }
                (end < bytes.len() && bytes[end] == b'"').then_some(end)
            });
            if let Some(quote) = raw {
                let hashes = quote - raw_start.unwrap();
                pos = quote + 1;
                while pos < bytes.len() {
                    if bytes[pos] == b'"' && bytes.get(pos + 1..pos + 1 + hashes).is_some_and(|tail| tail.iter().all(|byte| *byte == b'#')) {
                        pos += 1 + hashes; break;
                    }
                    pos += 1;
                }
            } else {
                let quote = if bytes[pos] == b'"' { Some(pos) }
                    else if bytes[pos] == b'b' && bytes.get(pos + 1).is_some_and(|byte| *byte == b'"' || *byte == b'\'') { Some(pos + 1) }
                    else if bytes[pos] == b'\'' { Some(pos) } else { None };
                let literal_end = quote.and_then(|quote| {
                    if bytes[quote] == b'\'' {
                        let first = *bytes.get(quote + 1)?;
                        let mut end = quote + 1;
                        if first == b'\\' {
                            end += 2;
                            if bytes.get(end - 1) == Some(&b'u') && bytes.get(end) == Some(&b'{') {
                                while end < bytes.len() && bytes[end] != b'}' { end += 1; }
                                end += 1;
                            } else if bytes.get(end - 1) == Some(&b'x') { end += 2; }
                        } else {
                            end += source.get(end..)?.chars().next()?.len_utf8();
                        }
                        return (bytes.get(end) == Some(&b'\'')).then_some(end + 1);
                    }
                    let mut end = quote + 1;
                    while end < bytes.len() && bytes[end] != b'\n' {
                        if bytes[end] == b'\\' { end += 2; continue; }
                        if bytes[end] == bytes[quote] { return Some(end + 1); }
                        end += 1;
                    }
                    None
                });
                if let Some(end) = literal_end { pos = end; }
                else if bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_' {
                    pos += 1;
                    while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') { pos += 1; }
                    tokens.push(Token { text: source[start..pos].into(), line: start_line });
                } else {
                    pos += if bytes[pos..].starts_with(b"::") { 2 } else { 1 };
                    if source.is_char_boundary(start) && source.is_char_boundary(pos) {
                        tokens.push(Token { text: source[start..pos].into(), line: start_line });
                    }
                }
            }
        }
        line += bytes[start..pos].iter().filter(|byte| **byte == b'\n').count();
    }
    tokens
}

fn matching(tokens: &[Token], start: usize, open: &str, close: &str) -> usize {
    let mut depth = 0;
    for (index, token) in tokens.iter().enumerate().skip(start) {
        if token.text == open { depth += 1; }
        if token.text == close { depth -= 1; if depth == 0 { return index; } }
    }
    panic!("unmatched {open} on line {}", tokens[start].line);
}

fn is_test_cfg(tokens: &[Token]) -> bool {
    let attr: Vec<&str> = tokens.iter().map(|token| token.text.as_str()).collect();
    if attr.starts_with(&["cfg", "(", "test", ")"]) { return true; }
    if !attr.starts_with(&["cfg", "(", "any", "("]) { return false; }
    let mut depth = 0;
    for token in &attr[4..] {
        match *token {
            "(" => depth += 1,
            ")" if depth == 0 => break,
            ")" => depth -= 1,
            "test" if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

fn without_tests(tokens: &[Token]) -> Vec<Token> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index].text == "#" && tokens.get(index + 1).is_some_and(|token| token.text == "[") {
            let end = matching(tokens, index + 1, "[", "]");
            if is_test_cfg(&tokens[index + 2..end]) {
                index = end + 1;
                let mut parens = 0;
                let mut brackets = 0;
                while index < tokens.len() {
                    match tokens[index].text.as_str() {
                        "(" => parens += 1, ")" => parens -= 1,
                        "[" => brackets += 1, "]" => brackets -= 1,
                        ";" if parens == 0 && brackets == 0 => { index += 1; break; }
                        "{" if parens == 0 && brackets == 0 => { index = matching(tokens, index, "{", "}") + 1; break; }
                        _ => {}
                    }
                    index += 1;
                }
                continue;
            }
        }
        if tokens[index].text == "mod" && tokens.get(index + 1).is_some_and(|token| matches!(token.text.as_str(), "testing" | "testkit")) {
            if tokens.get(index + 2).is_some_and(|token| token.text == "{") {
                index = matching(tokens, index + 2, "{", "}") + 1;
            } else { index += 3; }
            continue;
        }
        result.push(tokens[index].clone()); index += 1;
    }
    result
}

type UsePath = (Vec<String>, Option<String>);

fn expand_use(tokens: &[Token], index: &mut usize, prefix: &[String], out: &mut Vec<UsePath>) {
    let mut path = prefix.to_vec();
    while *index < tokens.len() {
        match tokens[*index].text.as_str() {
            "{" => {
                *index += 1;
                while *index < tokens.len() && tokens[*index].text != "}" {
                    if tokens[*index].text == "," { *index += 1; continue; }
                    expand_use(tokens, index, &path, out);
                }
                *index += 1; return;
            }
            "," | "}" | ";" => { if !path.is_empty() { out.push((path, None)); } return; }
            "as" => {
                *index += 1;
                let alias = tokens.get(*index).map(|token| token.text.clone());
                if *index < tokens.len() { *index += 1; }
                if !path.is_empty() { out.push((path, alias)); }
                return;
            }
            "::" => *index += 1,
            name => { path.push(name.to_owned()); *index += 1; }
        }
    }
    if !path.is_empty() { out.push((path, None)); }
}

fn paths(tokens: &[Token], module: &[String]) -> Vec<(String, usize)> {
    let mut result = Vec::new();
    let mut imported = BTreeMap::new();
    let mut scopes: Vec<Option<String>> = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if token.text == "use" {
            let line = token.line;
            let mut groups = Vec::new();
            index += 1;
            expand_use(tokens, &mut index, &[], &mut groups);
            for (path, alias) in groups {
                let mut found = Vec::new();
                record_path(&path, line, module, &scopes, &mut found);
                if let Some((target, _)) = found.first() {
                    let target_name = target.trim_start_matches("crate::");
                    let module_only = path.last().is_some_and(|name| name == "self" || name == target_name);
                    if module_only {
                        let name = alias.unwrap_or_else(|| target.trim_start_matches("crate::").to_owned());
                        imported.insert(name, target.clone());
                    }
                }
                result.extend(found);
            }
            continue;
        }
        if token.text == "crate" || token.text == "super" {
            let mut path = vec![token.text.clone()];
            let mut end = index + 1;
            while end + 1 < tokens.len() && tokens[end].text == "::" {
                path.push(tokens[end + 1].text.clone()); end += 2;
            }
            record_path(&path, token.line, module, &scopes, &mut result);
            index = end; continue;
        }
        if tokens.get(index + 1).is_some_and(|next| next.text == "::") {
            if let Some(target) = imported.get(&token.text) { result.push((target.clone(), token.line)); }
        }
        if token.text == "{" {
            let name = if index >= 2 && tokens[index - 2].text == "mod" { Some(tokens[index - 1].text.clone()) } else { None };
            scopes.push(name);
        } else if token.text == "}" { scopes.pop(); }
        index += 1;
    }
    result
}

fn record_path(path: &[String], line: usize, module: &[String], scopes: &[Option<String>], out: &mut Vec<(String, usize)>) {
    let mut resolved = if path.first().is_some_and(|part| part == "crate") { Vec::new() }
        else if path.first().is_some_and(|part| part == "super") {
            let mut current = module.to_vec();
            current.extend(scopes.iter().flatten().cloned());
            current
        } else { return };
    for part in path.iter().skip(usize::from(path.first().is_some_and(|part| part == "crate"))) {
        if part == "super" { resolved.pop(); }
        else if part != "self" { resolved.push(part.clone()); }
    }
    if let Some(target) = resolved.first() {
        if target != "*" && target != &module[0] { out.push((format!("crate::{target}"), line)); }
    }
}

fn source_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy();
        if kind.is_dir() {
            if !matches!(name.as_ref(), "bin" | "tests" | "testing" | "testkit") { source_files(&path, files); }
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "rs") && !matches!(name.as_ref(), "lib.rs" | "tests.rs" | "test_facts.rs" | "testing.rs" | "testkit.rs") && !name.ends_with("_tests.rs") {
            files.push(path);
        }
    }
}

fn test_module_files(file: &Path, tokens: &[Token]) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut index = 0;
    while index + 1 < tokens.len() {
        if tokens[index].text == "#" && tokens[index + 1].text == "[" {
            let end = matching(tokens, index + 1, "[", "]");
            if is_test_cfg(&tokens[index + 2..end]) {
                let mut next = end + 1;
                while next + 1 < tokens.len() && tokens[next].text == "#" && tokens[next + 1].text == "[" {
                    next = matching(tokens, next + 1, "[", "]") + 1;
                }
                while next < tokens.len() && matches!(tokens[next].text.as_str(), "pub" | "unsafe") {
                    next += 1;
                    if tokens.get(next).is_some_and(|token| token.text == "(") { next = matching(tokens, next, "(", ")") + 1; }
                }
                if tokens.get(next).is_some_and(|token| token.text == "mod") && tokens.get(next + 2).is_some_and(|token| token.text == ";") {
                    let dir = if matches!(file.file_name().unwrap().to_str(), Some("mod.rs" | "lib.rs")) { file.parent().unwrap().to_path_buf() } else { file.with_extension("") };
                    let name = &tokens[next + 1].text;
                    result.push(dir.join(format!("{name}.rs")));
                    result.push(dir.join(name));
                }
            }
            index = end;
        }
        index += 1;
    }
    result
}

fn module_path(path: &Path) -> Vec<String> {
    let relative = path.strip_prefix("src").unwrap();
    let mut parts: Vec<String> = relative.iter().map(|part| part.to_string_lossy().into_owned()).collect();
    let last = parts.pop().unwrap();
    if last != "mod.rs" { parts.push(last.trim_end_matches(".rs").to_owned()); }
    parts
}

#[test]
fn architecture() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    let source_dir = root.join("src");
    assert!(fs::symlink_metadata(&source_dir).unwrap().file_type().is_dir());
    source_files(&source_dir, &mut files);
    files.sort();
    let mut test_paths = Vec::new();
    for file in &files {
        test_paths.extend(test_module_files(file, &lex(&fs::read_to_string(file).unwrap())));
    }
    let lib = source_dir.join("lib.rs");
    assert!(fs::symlink_metadata(&lib).unwrap().file_type().is_file());
    test_paths.extend(test_module_files(&lib, &lex(&fs::read_to_string(&lib).unwrap())));
    let mut violations: BTreeMap<(String, String, String), Vec<usize>> = BTreeMap::new();
    for file in files {
        if test_paths.iter().any(|path| file.starts_with(path)) { continue; }
        let relative = file.strip_prefix(root).unwrap();
        let module = module_path(relative);
        let context = module[0].as_str();
        assert!(EDGES.iter().any(|(name, _)| *name == context), "{} is in no known context folder", relative.display());
        let allowed = EDGES.iter().find(|(name, _)| *name == context).unwrap().1;
        for (target, line) in paths(&without_tests(&lex(&fs::read_to_string(&file).unwrap())), &module) {
            let target_context = target.trim_start_matches("crate::");
            if target_context == "testkit" { continue; }
            assert!(EDGES.iter().any(|(name, _)| *name == target_context), "{}:{line}: {target} is in no known context folder", relative.display());
            if target_context != context && !allowed.contains(&target_context) {
                violations.entry((context.into(), relative.to_string_lossy().into_owned(), target)).or_default().push(line);
            }
        }
    }
    let tests_dir = root.join("tests");
    assert!(fs::symlink_metadata(&tests_dir).unwrap().file_type().is_dir());
    let baseline_dir = tests_dir.join("architecture_baseline");
    if let Ok(metadata) = fs::symlink_metadata(&baseline_dir) { assert!(metadata.file_type().is_dir()); }
    if std::env::var_os("ARCH_BASELINE").is_some_and(|value| value == "write") {
        fs::create_dir_all(&baseline_dir).unwrap();
        for (context, _) in EDGES {
            let mut contents = String::new();
            for ((owner, file, path), lines) in &violations {
                if owner == context { contents.push_str(&format!("T1 | {file} | {path} | {}\n", lines.len())); }
            }
            let file = baseline_dir.join(format!("{context}.txt"));
            if let Ok(metadata) = fs::symlink_metadata(&file) { assert!(metadata.file_type().is_file()); }
            fs::write(file, contents).unwrap();
        }
        return;
    }
    let mut expected = BTreeSet::new();
    for entry in fs::read_dir(&baseline_dir).unwrap() {
        let entry = entry.unwrap();
        assert!(entry.file_type().unwrap().is_file(), "baseline must be a regular file: {}", entry.path().display());
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(EDGES.iter().any(|(context, _)| name == format!("{context}.txt")), "unexpected baseline file: {name}");
    }
    for (context, _) in EDGES {
        let file = baseline_dir.join(format!("{context}.txt"));
        let contents = fs::read_to_string(&file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
        for line in contents.lines() {
            assert!(expected.insert((context.to_string(), line.to_string())), "duplicate baseline line: {line}");
        }
    }
    let mut errors = Vec::new();
    for ((context, file, path), lines) in &violations {
        let entry = format!("T1 | {file} | {path} | {}", lines.len());
        if !expected.remove(&(context.clone(), entry)) { errors.push(format!("{file}:{}: T1 unexpected {path} ({} occurrences)", lines[0], lines.len())); }
    }
    for (context, entry) in expected { errors.push(format!("stale {context} baseline: {entry}")); }
    assert!(errors.is_empty(), "architecture violations:\n{}", errors.join("\n"));
}

#[test]
fn lexer_removes_comments_and_literals_but_keeps_lifetimes() {
    let source = r###"use crate::real; // crate::line
/* crate::block /* crate::nested */ */ /// crate::doc
let a = "crate::string \\\""; let b = b"crate::bytes";
let c = r##"crate::raw \""##; let d = br#"crate::raw_bytes"#;
let e = 'x'; let f = b'\n'; let g = '\''; let h = ' '; let i = ','; let j = '\u{1f600}'; fn f<'a>(x: &'a str) { crate::live(); }
"###;
    let tokens = lex(source);
    let text = tokens.iter().map(|token| token.text.as_str()).collect::<Vec<_>>().join(" ");
    assert!(text.contains("crate :: real"));
    assert!(text.contains("crate :: live"));
    assert!(text.contains("' a"));
    for absent in ["line", "block", "nested", "doc", "string", "bytes", "raw", "raw_bytes"] { assert!(!text.contains(absent), "{text}"); }
}

#[test]
fn expands_nested_use_groups_and_aliases() {
    let tokens = lex("crate::a::{b::{C, D as Other}, e::F, self};");
    let mut index = 0;
    let mut result = Vec::new();
    expand_use(&tokens, &mut index, &[], &mut result);
    let rendered: Vec<_> = result.iter().map(|(path, alias)| (path.join("::"), alias.as_deref())).collect();
    assert_eq!(rendered, vec![("crate::a::b::C".into(), None), ("crate::a::b::D".into(), Some("Other")), ("crate::a::e::F".into(), None), ("crate::a::self".into(), None)]);
}

#[test]
fn skips_test_items_and_resolves_super() {
    let tokens = without_tests(&lex("#[cfg(test)] mod tests { crate::hidden(); } #[cfg(any(test, feature = \"testing\"))] fn f() { crate::hidden(); } mod testing { crate::hidden(); } #[cfg(any(not(test), feature = \"testing\"))] fn live() { crate::visible(); } mod nested { use super::super::{other::Thing, third}; }"));
    let found = paths(&tokens, &["one".into()]);
    assert_eq!(found, vec![("crate::visible".into(), 1), ("crate::other".into(), 1), ("crate::third".into(), 1)]);
}

#[test]
fn collects_bare_imported_modules_and_test_files() {
    let source = "#[cfg(test)] pub(crate) mod helpers; use crate::other as renamed; use crate::third::{self, Item}; renamed::run(); third::run();";
    let tokens = lex(source);
    assert_eq!(test_module_files(Path::new("src/example.rs"), &tokens), vec![PathBuf::from("src/example/helpers.rs"), PathBuf::from("src/example/helpers")]);
    assert_eq!(test_module_files(Path::new("src/lib.rs"), &tokens), vec![PathBuf::from("src/helpers.rs"), PathBuf::from("src/helpers")]);
    assert_eq!(paths(&without_tests(&tokens), &["example".into()]), vec![("crate::other".into(), 1), ("crate::third".into(), 1), ("crate::third".into(), 1), ("crate::other".into(), 1), ("crate::third".into(), 1)]);
}
