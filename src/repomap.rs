//! A map of the repository for the model: each source file with its top-level symbols,
//! cut to a size budget. Files come from `git ls-files` (so `.gitignore` is honoured) or,
//! outside git, from a walk that skips build and dependency folders. Symbols are found by
//! line shape, not by a parser: cheap, language-agnostic enough, and never fails.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Files larger than this are not read for symbols.
const MAX_FILE_BYTES: u64 = 400_000;

/// Folders a walk never enters.
const SKIP_DIRS: [&str; 9] = ["target", "node_modules", "dist", "build", "vendor", "__pycache__", "venv", ".venv", "out"];

/// Most files a map will consider.
const MAX_FILES: usize = 5_000;

/// The language of a source file, by extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Rust,
    Python,
    Script,
    Go,
}

impl Lang {
    fn of(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "rs" => Some(Self::Rust),
            "py" => Some(Self::Python),
            "js" | "jsx" | "ts" | "tsx" | "mjs" => Some(Self::Script),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    /// The declaration words this language opens a symbol with, after any `pub`/`export`.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &["fn", "struct", "enum", "trait", "mod", "type", "const"],
            Self::Python => &["def", "class"],
            Self::Script => &["function", "class", "interface", "type", "const", "enum"],
            Self::Go => &["func", "type"],
        }
    }
}

/// A top-level declaration: its keyword and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub keyword: &'static str,
    pub name: String,
}

/// One source file and what it declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMap {
    pub path: PathBuf,
    pub symbols: Vec<Symbol>,
}

fn identifier(rest: &str) -> String {
    rest.trim_start().trim_start_matches(['(', '*']).chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect()
}

/// The words that may stand before a declaration keyword.
const MODIFIERS: [&str; 6] = ["pub", "export", "default", "async", "unsafe", "extern"];

/// The symbol a line declares at column 0 (Rust also at one indent level for `impl` methods
/// is deliberately left out: the map is for orientation, not completeness).
fn symbol_of(line: &str, lang: Lang) -> Option<Symbol> {
    if line.starts_with([' ', '\t']) {
        return None;
    }
    let mut words = line.split_whitespace().peekable();
    let mut head = words.next()?;
    // `pub(crate)` and friends count as `pub`.
    while MODIFIERS.contains(&head.split('(').next().unwrap_or(head)) {
        head = words.next()?;
    }
    let keyword = lang.keywords().iter().copied().find(|k| *k == head)?;
    let rest = &line[line.find(head)? + head.len()..];
    let name = identifier(rest);
    // A Go method is `func (r *T) Name(`: take the name after the receiver.
    let name = if lang == Lang::Go && keyword == "func" && rest.trim_start().starts_with('(') {
        let after = rest.split_once(')').map(|(_, r)| identifier(r)).unwrap_or_default();
        if after.is_empty() { name } else { after }
    } else {
        name
    };
    (!name.is_empty()).then_some(Symbol { keyword, name })
}

/// The symbols declared in `text`.
pub fn symbols(text: &str, path: &Path) -> Vec<Symbol> {
    let Some(lang) = Lang::of(path) else { return Vec::new() };
    text.lines().filter_map(|l| symbol_of(l, lang)).collect()
}

/// The project's files: tracked and untracked-but-not-ignored when `dir` is in a git work
/// tree, else a walk.
fn list_files(dir: &Path) -> Vec<PathBuf> {
    let git = Command::new("git").arg("-C").arg(dir).args(crate::gitflow::no_repo_programs(dir)).args(["ls-files", "-co", "--exclude-standard"]).stdin(Stdio::null()).stderr(Stdio::null()).output();
    if let Ok(out) = git {
        if out.status.success() {
            return String::from_utf8_lossy(&out.stdout).lines().map(PathBuf::from).take(MAX_FILES).collect();
        }
    }
    let mut found = Vec::new();
    walk(dir, Path::new(""), &mut found);
    found
}

fn walk(root: &Path, rel: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root.join(rel)) else { return };
    let mut names: Vec<_> = entries.flatten().collect();
    names.sort_by_key(|e| e.file_name());
    for entry in names {
        if found.len() >= MAX_FILES {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else { continue };
        let path = rel.join(&name);
        if kind.is_dir() {
            if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                walk(root, &path, found);
            }
        } else if kind.is_file() {
            found.push(path);
        }
    }
}

/// The map of `dir`: every source file with at least one symbol, in path order.
pub fn build(dir: &Path) -> Vec<FileMap> {
    let mut files: Vec<PathBuf> = list_files(dir).into_iter().filter(|p| Lang::of(p).is_some()).collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let full = dir.join(&path);
            let small = std::fs::metadata(&full).is_ok_and(|m| m.len() <= MAX_FILE_BYTES);
            let text = if small { std::fs::read_to_string(&full).ok()? } else { return None };
            let symbols = symbols(&text, &path);
            (!symbols.is_empty()).then_some(FileMap { path, symbols })
        })
        .collect()
}

/// How well a file answers the `query` words (all lower case): its path counts most, then
/// each symbol name. Zero means unrelated; with no query every file ranks equally.
fn score(file: &FileMap, query: &[String]) -> usize {
    let path = file.path.to_string_lossy().to_lowercase();
    query.iter().map(|w| usize::from(path.contains(w.as_str())) * 10 + file.symbols.iter().filter(|s| s.name.to_lowercase().contains(w.as_str())).count()).sum()
}

fn render_file(file: &FileMap, max_symbols: usize) -> String {
    let shown: Vec<String> = file.symbols.iter().take(max_symbols).map(|s| format!("{} {}", s.keyword, s.name)).collect();
    let more = file.symbols.len().saturating_sub(max_symbols);
    let tail = if more > 0 { format!(", +{more} more") } else { String::new() };
    format!("{}\n  {}{tail}", file.path.display(), shown.join(", "))
}

/// The map as text, at most about `budget` characters. With a `query` the files that match
/// it come first (and unrelated ones are dropped); otherwise the files with the most symbols
/// do, listed in path order.
pub fn render(map: &[FileMap], query: &[String], budget: usize) -> String {
    let words: Vec<String> = query.iter().map(|w| w.to_lowercase()).collect();
    let mut ranked: Vec<(usize, &FileMap)> = map.iter().map(|f| (score(f, &words), f)).filter(|(s, _)| words.is_empty() || *s > 0).collect();
    // Stable: equal scores keep path order. Best first, to decide what fits.
    ranked.sort_by_key(|(s, f)| (std::cmp::Reverse(*s), std::cmp::Reverse(f.symbols.len())));
    let mut used = 0;
    let mut chosen: Vec<&FileMap> = Vec::new();
    for (_, file) in &ranked {
        let cost = render_file(file, MAX_SYMBOLS_PER_FILE).len() + 1;
        if used + cost > budget && !chosen.is_empty() {
            break;
        }
        used += cost;
        chosen.push(file);
    }
    let left_out = ranked.len() - chosen.len();
    chosen.sort_by(|a, b| a.path.cmp(&b.path));
    let mut text = chosen.iter().map(|f| render_file(f, MAX_SYMBOLS_PER_FILE)).collect::<Vec<_>>().join("\n");
    if left_out > 0 {
        text.push_str(&format!("\n… {left_out} more file(s) not shown; ask with words to narrow the map"));
    }
    text
}

/// Symbols listed per file before "+N more".
const MAX_SYMBOLS_PER_FILE: usize = 12;

/// How the map goes with the next message.
pub fn note(map_text: &str) -> String {
    format!("Map of this repository (source files and their top-level symbols), for orientation:\n```\n{map_text}\n```")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projectkind::tempdir::Dir;

    fn names(text: &str, file: &str) -> Vec<String> {
        symbols(text, Path::new(file)).into_iter().map(|s| format!("{} {}", s.keyword, s.name)).collect()
    }

    #[test]
    fn rust_symbols_are_top_level_declarations_with_any_visibility() {
        let src = "pub struct Engine {\n    fn inner() {}\n}\npub(crate) async fn run_turn() {}\nenum Mode {}\nuse std::fmt;\nconst MAX: u32 = 3;\n";
        assert_eq!(names(src, "a.rs"), ["struct Engine", "fn run_turn", "enum Mode", "const MAX"]);
    }

    #[test]
    fn python_script_and_go_symbols() {
        assert_eq!(names("def main():\n    pass\nclass Repo:\n    def m(self): ...\n", "a.py"), ["def main", "class Repo"]);
        assert_eq!(names("export default function App() {}\nexport const x = 1;\nclass A {}\n", "a.tsx"), ["function App", "const x", "class A"]);
        assert_eq!(names("func (s *Server) Handle(w int) {}\nfunc main() {}\ntype Conf struct{}\n", "a.go"), ["func Handle", "func main", "type Conf"]);
        assert!(names("def x(): pass", "notes.txt").is_empty());
    }

    #[test]
    fn a_walk_skips_build_folders_and_files_without_symbols() {
        let d = Dir::new();
        d.write("src/lib.rs", "pub fn alpha() {}\n");
        d.write("src/empty.rs", "// nothing\n");
        d.write("target/debug/gen.rs", "pub fn generated() {}\n");
        d.write("node_modules/x/index.js", "function dep() {}\n");
        d.write("README.md", "# hi\n");
        let map = build(d.path());
        assert_eq!(map.iter().map(|f| f.path.to_string_lossy().into_owned()).collect::<Vec<_>>(), ["src/lib.rs"]);
    }

    #[test]
    fn a_query_puts_matching_files_first_and_drops_the_rest() {
        let d = Dir::new();
        d.write("src/router.rs", "pub fn pick() {}\n");
        d.write("src/ui.rs", "pub fn draw_router_badge() {}\n");
        d.write("src/other.rs", "pub fn unrelated() {}\n");
        let map = build(d.path());
        let text = render(&map, &["router".to_string()], 10_000);
        assert!(text.contains("src/router.rs") && text.contains("src/ui.rs") && !text.contains("other.rs"), "{text}");
        let all = render(&map, &[], 10_000);
        assert!(all.contains("src/other.rs"));
    }

    #[test]
    fn the_budget_cuts_the_map_and_says_so() {
        let d = Dir::new();
        for i in 0..20 {
            d.write(&format!("src/m{i}.rs"), &format!("pub fn f{i}() {{}}\n"));
        }
        let text = render(&build(d.path()), &[], 200);
        assert!(text.len() < 400, "{}", text.len());
        assert!(text.contains("more file(s) not shown"), "{text}");
    }

    #[test]
    fn many_symbols_in_one_file_are_summarised() {
        let src: String = (0..20).map(|i| format!("fn f{i}() {{}}\n")).collect();
        let f = FileMap { path: "a.rs".into(), symbols: symbols(&src, Path::new("a.rs")) };
        assert!(render_file(&f, 12).ends_with("+8 more"));
    }

    #[test]
    fn git_hides_ignored_files() {
        let d = Dir::new();
        let git = |args: &[&str]| Command::new("git").arg("-C").arg(d.path()).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        if !git(&["init", "-q"]) {
            return; // no git here: the walk is covered above
        }
        d.write(".gitignore", "secret/\n");
        d.write("secret/key.rs", "pub fn hidden() {}\n");
        d.write("src/a.rs", "pub fn shown() {}\n");
        let files: Vec<String> = build(d.path()).iter().map(|f| f.path.to_string_lossy().into_owned()).collect();
        assert_eq!(files, ["src/a.rs"]);
    }
}
