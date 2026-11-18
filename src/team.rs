//! Shared repo rules: `.jev/team.json` in the project, committed so the whole team routes and
//! works the same way. Two things live there today:
//!
//! - `routing`: rules that send a prompt to a provider or model when it mentions something
//!   (`{"contains": "migration", "pin": "claude:claude-opus-5-5"}`), used only while the
//!   person has not pinned a model themselves, and
//! - `max_profile`: the most trusting permission profile the repo allows.
//!
//! Loading is strict about what it can check (a pin must be one `/model` accepts) and says
//! which rule is wrong, so a typo in a shared file is found by whoever edits it.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::profile::Profile;
use crate::routing::Pin;

/// Where the file is, relative to the project.
pub const FILE: &str = ".jev/team.json";

#[derive(Debug, Deserialize)]
struct RawRule {
    contains: String,
    pin: String,
}

#[derive(Debug, Deserialize)]
struct RawTeam {
    #[serde(default)]
    routing: Vec<RawRule>,
    #[serde(default)]
    max_profile: Option<Profile>,
}

/// One routing rule: a prompt that contains `contains` (any case) goes to `pin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub contains: String,
    pub pin: Pin,
}

/// The shared rules of a project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Team {
    pub routing: Vec<Rule>,
    pub max_profile: Option<Profile>,
}

/// Why a team file could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamError(pub String);

impl std::fmt::Display for TeamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{FILE}: {}", self.0)
    }
}

pub fn path_in(project: &Path) -> PathBuf {
    project.join(FILE)
}

impl Team {
    /// Read rules from the text of a team file.
    pub fn parse(text: &str) -> Result<Self, TeamError> {
        let raw: RawTeam = serde_json::from_str(text).map_err(|e| TeamError(e.to_string()))?;
        let mut routing = Vec::new();
        for (i, r) in raw.routing.into_iter().enumerate() {
            if r.contains.trim().is_empty() {
                return Err(TeamError(format!("routing rule {} has an empty \"contains\"", i + 1)));
            }
            let pin = Pin::parse(&r.pin).map_err(|e| TeamError(format!("routing rule {}: {e}", i + 1)))?;
            routing.push(Rule { contains: r.contains.trim().to_ascii_lowercase(), pin });
        }
        Ok(Team { routing, max_profile: raw.max_profile })
    }

    /// The project's rules; none when the file does not exist.
    pub fn load(project: &Path) -> Result<Self, TeamError> {
        match std::fs::read_to_string(path_in(project)) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Team::default()),
            Err(e) => Err(TeamError(e.to_string())),
        }
    }

    /// The pin of the first rule the prompt matches.
    pub fn pin_for(&self, prompt: &str) -> Option<&Pin> {
        let lower = prompt.to_ascii_lowercase();
        self.routing.iter().find(|r| lower.contains(&r.contains)).map(|r| &r.pin)
    }

    pub fn is_empty(&self) -> bool {
        self.routing.is_empty() && self.max_profile.is_none()
    }

    /// `/team`: what the shared file says.
    pub fn text(&self) -> String {
        if self.is_empty() {
            return format!("  No shared rules: create {FILE} to route and limit this repo for the whole team.");
        }
        let mut lines = vec![format!("  Shared rules from {FILE}")];
        match self.max_profile {
            Some(p) => lines.push(format!("  max profile: {p} ({})", p.about())),
            None => lines.push("  max profile: not limited".to_string()),
        }
        for r in &self.routing {
            lines.push(format!("  prompts containing \"{}\" go to {}", r.contains, r.pin));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_parse_and_the_first_match_wins_in_any_case() {
        let team = Team::parse(
            r#"{"routing":[{"contains":"Migration","pin":"claude:claude-opus-5-5"},{"contains":"typo","pin":"codex"}],"max_profile":"standard","future_key":1}"#,
        )
        .unwrap();
        assert_eq!(team.max_profile, Some(Profile::Standard));
        assert_eq!(team.pin_for("Write the DB MIGRATION and fix a typo").map(|p| p.to_string()), Some("claude:claude-opus-5-5".to_string()));
        assert_eq!(team.pin_for("fix a typo").map(|p| p.to_string()), Some("codex".to_string()));
        assert_eq!(team.pin_for("add a button"), None);
    }

    #[test]
    fn a_bad_rule_is_named_by_its_number() {
        let e = Team::parse(r#"{"routing":[{"contains":"a","pin":"codex"},{"contains":"b","pin":"nonsense"}]}"#).unwrap_err();
        assert!(e.0.contains("routing rule 2"), "{e}");
        let e = Team::parse(r#"{"routing":[{"contains":" ","pin":"codex"}]}"#).unwrap_err();
        assert!(e.0.contains("empty"), "{e}");
        assert!(Team::parse(r#"{"max_profile":"yolo"}"#).is_err());
        assert!(Team::parse("not json").is_err());
    }

    #[test]
    fn a_missing_file_is_no_rules_and_a_present_one_loads() {
        let dir = std::env::temp_dir().join(format!("jev-team-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".jev")).unwrap();
        assert_eq!(Team::load(&dir.join("nowhere")), Ok(Team::default()));
        std::fs::write(path_in(&dir), r#"{"max_profile":"careful"}"#).unwrap();
        assert_eq!(Team::load(&dir).unwrap().max_profile, Some(Profile::Careful));
        std::fs::write(path_in(&dir), "{").unwrap();
        assert!(Team::load(&dir).unwrap_err().to_string().starts_with(".jev/team.json: "));
    }

    #[test]
    fn the_text_lists_limit_and_rules_or_says_how_to_start() {
        assert!(Team::default().text().contains("create .jev/team.json"));
        let t = Team::parse(r#"{"routing":[{"contains":"docs","pin":"codex"}],"max_profile":"careful"}"#).unwrap();
        let text = t.text();
        assert!(text.contains("max profile: careful") && text.contains("\"docs\" go to codex"), "{text}");
    }
}
