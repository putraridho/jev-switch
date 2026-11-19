//! A unified diff as data: files, hunks and typed lines, and the way back to patch text.
//!
//! The renderer (`uifmt::diffview`), the per-hunk review and the patch export all work on
//! these types, so none of them re-parses text. Parsing is by the counts in each `@@`
//! header, so a removed line that looks like a file header (`--- x`) stays a line.

/// What a diff line does to the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file`: belongs to the line before it.
    NoNewline,
}

impl LineKind {
    /// The first character of the line in patch text.
    pub fn marker(self) -> char {
        match self {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
            LineKind::NoNewline => '\\',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// The text without the marker and without the newline.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ -a,b +c,d @@ section` line exactly as it was.
    pub header: String,
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    pub fn added(&self) -> usize {
        self.count(LineKind::Added)
    }

    pub fn removed(&self) -> usize {
        self.count(LineKind::Removed)
    }

    fn count(&self, kind: LineKind) -> usize {
        self.lines.iter().filter(|l| l.kind == kind).count()
    }

    fn push_text(&self, out: &mut String) {
        out.push_str(&self.header);
        out.push('\n');
        for l in &self.lines {
            out.push(l.kind.marker());
            out.push_str(&l.text);
            out.push('\n');
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// The path after the change (the new one for a rename).
    pub path: String,
    /// Every line before the first hunk (`diff --git`, `index`, `---`, `+++`, ...).
    pub header: Vec<String>,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    pub fn added(&self) -> usize {
        self.hunks.iter().map(Hunk::added).sum()
    }

    pub fn removed(&self) -> usize {
        self.hunks.iter().map(Hunk::removed).sum()
    }

    /// Patch text for the whole file.
    pub fn to_patch(&self) -> String {
        self.patch_of(self.hunks.iter())
    }

    /// Patch text holding only hunk `index` (applies with `git apply`), `None` out of range.
    /// Hunks keep their own line numbers, which `git apply` locates by content.
    pub fn hunk_patch(&self, index: usize) -> Option<String> {
        self.hunks.get(index).map(|h| self.patch_of(std::iter::once(h)))
    }

    fn patch_of<'a>(&self, hunks: impl Iterator<Item = &'a Hunk>) -> String {
        let mut out: String = self.header.iter().map(|l| format!("{l}\n")).collect();
        for h in hunks {
            h.push_text(&mut out);
        }
        out
    }
}

/// `@@ -12,3 +14,4 @@` gives `(12, 3, 14, 4)`; a missing count is 1.
fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let new = rest.split_once(" @@").map_or(rest, |(n, _)| n);
    let span = |s: &str| -> Option<(u32, u32)> {
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (os, oc) = span(old)?;
    let (ns, nc) = span(new)?;
    Some((os, oc, ns, nc))
}

fn unquote_path(p: &str) -> &str {
    p.trim_matches('"')
}

fn path_of_header(header: &[String]) -> String {
    let side = |prefix: &str| header.iter().find_map(|l| l.strip_prefix(prefix)).map(|p| unquote_path(p.split('\t').next().unwrap_or(p)));
    // Git quotes a name with a quote, a backslash or a control character: `+++ "b/na\"me"`.
    for (plain, quoted) in [("+++ b/", "+++ \"b/"), ("--- a/", "--- \"a/")] {
        if let Some(p) = side(plain).or_else(|| side(quoted)) {
            return p.to_string();
        }
    }
    // A pure rename, a mode change or a binary file has no ---/+++ lines.
    if let Some(p) = header.iter().find_map(|l| l.strip_prefix("rename to ")) {
        return p.to_string();
    }
    header
        .first()
        .and_then(|l| l.strip_prefix("diff --git "))
        .and_then(|l| l.rsplit_once(" b/"))
        .map(|(_, p)| unquote_path(p).to_string())
        .unwrap_or_default()
}

/// Split diff text into files and hunks. Text that is not a diff gives no files.
pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with("diff ") {
            files.push(FileDiff { path: String::new(), header: vec![line.to_string()], hunks: Vec::new() });
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        let Some((old_start, mut old_left, new_start, mut new_left)) = parse_hunk_header(line) else {
            file.header.push(line.to_string());
            continue;
        };
        let mut hunk = Hunk { header: line.to_string(), old_start, new_start, lines: Vec::new() };
        while old_left > 0 || new_left > 0 || lines.peek().is_some_and(|l| l.starts_with('\\')) {
            let Some(raw) = lines.next() else { break };
            let (kind, text) = match raw.chars().next() {
                Some('+') => (LineKind::Added, &raw[1..]),
                Some('-') => (LineKind::Removed, &raw[1..]),
                Some('\\') => (LineKind::NoNewline, &raw[1..]),
                Some(' ') => (LineKind::Context, &raw[1..]),
                // An editor stripped the lone space of an empty context line.
                None => (LineKind::Context, ""),
                Some(_) => break,
            };
            match kind {
                LineKind::Context => {
                    old_left = old_left.saturating_sub(1);
                    new_left = new_left.saturating_sub(1);
                }
                LineKind::Removed => old_left = old_left.saturating_sub(1),
                LineKind::Added => new_left = new_left.saturating_sub(1),
                LineKind::NoNewline => {}
            }
            hunk.lines.push(DiffLine { kind, text: text.to_string() });
        }
        file.hunks.push(hunk);
    }
    for f in &mut files {
        f.path = path_of_header(&f.header);
    }
    files
}

/// The whole diff back as patch text (what `git apply` takes).
pub fn to_patch(files: &[FileDiff]) -> String {
    files.iter().map(FileDiff::to_patch).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,3 @@ fn main\n one\n-two\n+TWO\n three\n@@ -10 +10,2 @@\n ten\n+eleven\n\\ No newline at end of file\ndiff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hi\n";

    #[test]
    fn quoted_paths_are_read() {
        let text = "diff --git \"a/na\\\"me.txt\" \"b/na\\\"me.txt\"\nindex 1..2 100644\n--- \"a/na\\\"me.txt\"\n+++ \"b/na\\\"me.txt\"\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(parse(text)[0].path, "na\\\"me.txt");
    }

    #[test]
    fn splits_files_hunks_and_lines() {
        let files = parse(TWO);
        assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["src/a.rs", "new.txt"]);
        let a = &files[0];
        assert_eq!((a.hunks.len(), a.added(), a.removed()), (2, 2, 1));
        assert_eq!((a.hunks[1].old_start, a.hunks[1].new_start), (10, 10));
        assert_eq!(a.hunks[1].lines.last().map(|l| l.kind), Some(LineKind::NoNewline));
        assert_eq!(a.header[0], "diff --git a/src/a.rs b/src/a.rs");
    }

    #[test]
    fn round_trips_to_the_same_text() {
        assert_eq!(to_patch(&parse(TWO)), TWO);
    }

    #[test]
    fn one_hunk_patch_has_the_header_and_only_that_hunk() {
        let files = parse(TWO);
        let p = files[0].hunk_patch(1).unwrap();
        assert!(p.starts_with("diff --git a/src/a.rs") && p.contains("+eleven") && !p.contains("+TWO"), "{p}");
        assert_eq!(files[0].hunk_patch(2), None);
    }

    #[test]
    fn a_removed_line_that_looks_like_a_header_stays_a_line() {
        let text = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,1 @@\n--- not a header\n keep\n";
        let files = parse(text);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].hunks[0].removed(), 1);
        assert_eq!(files[0].hunks[0].lines[0].text, "-- not a header");
    }

    #[test]
    fn renames_and_binary_files_get_a_path_without_hunks() {
        let text = "diff --git a/old.txt b/new name.txt\nsimilarity index 100%\nrename from old.txt\nrename to new name.txt\ndiff --git a/i.png b/i.png\nBinary files a/i.png and b/i.png differ\n";
        let files = parse(text);
        assert_eq!(files[0].path, "new name.txt");
        assert_eq!(files[1].path, "i.png");
        assert!(files.iter().all(|f| f.hunks.is_empty()));
    }

    #[test]
    fn text_that_is_not_a_diff_gives_nothing() {
        assert!(parse("hello\nworld\n").is_empty());
        assert!(parse("").is_empty());
    }
}
