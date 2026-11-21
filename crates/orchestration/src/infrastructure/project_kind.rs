//! What kind of project a folder is (Cargo, npm, Python, Go) and the commands that test,
//! lint and build it. Detection reads a few marker files; nothing is run here.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::Deserialize;

/// A kind of project, by its marker file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Cargo,
    Npm,
    Python,
    Go,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cargo => "Rust (cargo)",
            Self::Npm => "JavaScript/TypeScript (package.json)",
            Self::Python => "Python",
            Self::Go => "Go",
        })
    }
}

/// What a check does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Test,
    Lint,
    Build,
}

impl Check {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "test" | "tests" => Some(Self::Test),
            "lint" => Some(Self::Lint),
            "build" => Some(Self::Build),
            _ => None,
        }
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Test => "test",
            Self::Lint => "lint",
            Self::Build => "build",
        })
    }
}

/// A detected project and its commands; a check the project has no command for is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub kind: Kind,
    pub test: Option<String>,
    pub lint: Option<String>,
    pub build: Option<String>,
}

impl Project {
    pub fn command(&self, check: Check) -> Option<&str> {
        match check {
            Check::Test => &self.test,
            Check::Lint => &self.lint,
            Check::Build => &self.build,
        }
        .as_deref()
    }
}

#[derive(Deserialize, Default)]
struct PackageJson {
    #[serde(default)]
    scripts: BTreeMap<String, String>,
}

fn read(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok()
}

fn cargo() -> Project {
    Project { kind: Kind::Cargo, test: Some("cargo test".into()), lint: Some("cargo clippy --all-targets".into()), build: Some("cargo build --all-targets".into()) }
}

fn npm(dir: &Path) -> Project {
    let scripts = read(dir, "package.json").and_then(|t| serde_json::from_str::<PackageJson>(&t).ok()).unwrap_or_default().scripts;
    let runner = if dir.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if dir.join("yarn.lock").is_file() {
        "yarn"
    } else {
        "npm"
    };
    let script = |name: &str| {
        scripts.contains_key(name).then(|| if name == "test" && runner == "npm" { "npm test".to_string() } else { format!("{runner} run {name}") })
    };
    Project { kind: Kind::Npm, test: script("test"), lint: script("lint"), build: script("build") }
}

fn python(dir: &Path) -> Project {
    let pyproject = read(dir, "pyproject.toml").unwrap_or_default();
    let ruff = pyproject.contains("[tool.ruff") || dir.join("ruff.toml").is_file() || dir.join(".ruff.toml").is_file();
    Project { kind: Kind::Python, test: Some("python3 -m pytest -q".into()), lint: ruff.then(|| "ruff check .".to_string()), build: None }
}

fn go() -> Project {
    Project { kind: Kind::Go, test: Some("go test ./...".into()), lint: Some("go vet ./...".into()), build: Some("go build ./...".into()) }
}

/// Every kind of project `dir` is, the first being the main one. A folder can be several
/// (a Rust crate with a web front end).
pub fn detect(dir: &Path) -> Vec<Project> {
    let has = |name: &str| dir.join(name).is_file();
    let mut found = Vec::new();
    if has("Cargo.toml") {
        found.push(cargo());
    }
    if has("package.json") {
        found.push(npm(dir));
    }
    if has("pyproject.toml") || has("pytest.ini") || has("setup.py") || has("setup.cfg") || has("requirements.txt") {
        found.push(python(dir));
    }
    if has("go.mod") {
        found.push(go());
    }
    found
}

#[cfg(test)]
mod tests {
    use jev_testkit::temp_folder::Dir;
    use super::*;

    fn dir_with(files: &[(&str, &str)]) -> Dir {
        let d = Dir::new();
        for (name, text) in files {
            d.write(name, text);
        }
        d
    }

    #[test]
    fn cargo_go_and_python_are_found_by_their_marker() {
        let d = dir_with(&[("Cargo.toml", ""), ("go.mod", "")]);
        let kinds: Vec<Kind> = detect(d.path()).iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [Kind::Cargo, Kind::Go]);
        let py = dir_with(&[("pyproject.toml", "[tool.ruff]\nline-length = 100\n")]);
        let found = detect(py.path());
        assert_eq!(found[0].command(Check::Test), Some("python3 -m pytest -q"));
        assert_eq!(found[0].command(Check::Lint), Some("ruff check ."));
        assert_eq!(found[0].command(Check::Build), None);
    }

    #[test]
    fn npm_offers_only_the_scripts_the_package_has_and_follows_the_lockfile() {
        let d = dir_with(&[("package.json", r#"{"scripts":{"test":"jest","build":"tsc"}}"#), ("pnpm-lock.yaml", "")]);
        let p = &detect(d.path())[0];
        assert_eq!(p.test.as_deref(), Some("pnpm run test"));
        assert_eq!(p.lint, None);
        assert_eq!(p.build.as_deref(), Some("pnpm run build"));
        let plain = dir_with(&[("package.json", r#"{"scripts":{"test":"jest"}}"#)]);
        assert_eq!(detect(plain.path())[0].test.as_deref(), Some("npm test"));
        let broken = dir_with(&[("package.json", "{not json")]);
        assert_eq!(detect(broken.path())[0].test, None);
    }

    #[test]
    fn an_empty_folder_is_no_project_and_checks_parse() {
        assert!(detect(dir_with(&[]).path()).is_empty());
        assert_eq!(Check::parse("tests"), Some(Check::Test));
        assert_eq!(Check::parse("deploy"), None);
    }
}
