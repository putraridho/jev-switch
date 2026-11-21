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

use jev_approval::domain::profile::Profile;
use jev_routing::domain::decide::Pin;

/// Where the file is, relative to the project.
pub const FILE: &str = ".jev/team.json";

#[derive(Debug, Deserialize)]
struct RawRule {
    contains: String,
    pin: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "lowercase")]
enum RawProfile {
    Locked,
    Careful,
    Standard,
    Trusted,
}

impl From<RawProfile> for Profile {
    fn from(value: RawProfile) -> Self {
        match value {
            RawProfile::Locked => Self::Locked,
            RawProfile::Careful => Self::Careful,
            RawProfile::Standard => Self::Standard,
            RawProfile::Trusted => Self::Trusted,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawTeam {
    #[serde(default)]
    routing: Vec<RawRule>,
    #[serde(default)]
    max_profile: Option<RawProfile>,
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
        // An object only, as for the limit alone (serde would also take an array by position).
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| TeamError(e.to_string()))?;
        if !value.is_object() {
            return Err(TeamError("expected a JSON object".into()));
        }
        let raw: RawTeam = serde_json::from_value(value).map_err(|e| TeamError(e.to_string()))?;
        let mut routing = Vec::new();
        for (i, r) in raw.routing.into_iter().enumerate() {
            if r.contains.trim().is_empty() {
                return Err(TeamError(format!("routing rule {} has an empty \"contains\"", i + 1)));
            }
            let pin = Pin::parse(&r.pin).map_err(|e| TeamError(format!("routing rule {}: {e}", i + 1)))?;
            routing.push(Rule { contains: r.contains.trim().to_ascii_lowercase(), pin });
        }
        Ok(Team { routing, max_profile: raw.max_profile.map(Profile::from) })
    }

    /// Only the profile limit of a team file, read apart from the routing rules so that a rule
    /// this version does not understand cannot cost the limit (or lock a session out).
    pub fn parse_cap(text: &str) -> Result<Option<Profile>, TeamError> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| TeamError(e.to_string()))?;
        let Some(fields) = value.as_object() else {
            return Err(TeamError("expected a JSON object".into()));
        };
        match fields.get("max_profile") {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(v) => serde_json::from_value::<RawProfile>(v.clone())
                .map(|profile| Some(profile.into()))
                .map_err(|e| TeamError(format!("max_profile: {e}"))),
        }
    }

    /// The project's profile limit; none when the file does not exist or sets none.
    pub fn load_cap(project: &Path) -> Result<Option<Profile>, TeamError> {
        match std::fs::read_to_string(path_in(project)) {
            Ok(text) => Self::parse_cap(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(TeamError(e.to_string())),
        }
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
    fn serde_uses_the_lowercase_names() {
        let profile = serde_json::from_str::<RawProfile>("\"careful\"").unwrap();
        assert_eq!(Profile::from(profile), Profile::Careful);
        assert_eq!(serde_json::to_string(&RawProfile::Trusted).unwrap(), "\"trusted\"");
    }

    #[test]
    fn the_limit_survives_a_routing_rule_this_version_cannot_read() {
        let text = r#"{"routing":[{"contains":"x","pin":"nosuch:model-9"}],"max_profile":"careful"}"#;
        assert!(Team::parse(text).is_err());
        assert_eq!(Team::parse_cap(text), Ok(Some(Profile::Careful)));
        assert_eq!(Team::parse_cap(r#"{"routing":[]}"#), Ok(None));
        assert!(Team::parse_cap(r#"{"max_profile":"careful",}"#).is_err(), "broken JSON");
        assert!(Team::parse_cap(r#"{"max_profile":"godmode"}"#).is_err(), "unknown profile");
        for not_an_object in [r#"[{"max_profile":"careful"}]"#, r#""careful""#, "null", r#"[[], "trusted"]"#] {
            assert!(Team::parse_cap(not_an_object).is_err(), "{not_an_object}");
            assert!(Team::parse(not_an_object).is_err(), "{not_an_object}");
        }
    }

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
