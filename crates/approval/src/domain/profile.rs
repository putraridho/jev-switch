//! Permission profiles: four named levels of trust that each stand for one [`Mode`], so a
//! team can say "this repo is `standard`" instead of explaining modes. A profile is ordered
//! from the most to the least restrictive, which is what lets a shared team file cap it (see
//! [`jev_turn::application::team_rules`]).

use std::fmt;
use std::str::FromStr;

use crate::domain::mode::Mode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Profile {
    /// Read and plan only.
    Locked,
    /// Ask before every edit and command.
    Careful,
    /// Edits are accepted, commands are asked.
    Standard,
    /// Everything is accepted without asking.
    Trusted,
}

impl Profile {
    pub const ALL: [Profile; 4] = [Profile::Locked, Profile::Careful, Profile::Standard, Profile::Trusted];

    pub fn name(self) -> &'static str {
        match self {
            Profile::Locked => "locked",
            Profile::Careful => "careful",
            Profile::Standard => "standard",
            Profile::Trusted => "trusted",
        }
    }

    /// The mode this profile runs in.
    pub fn mode(self) -> Mode {
        match self {
            Profile::Locked => Mode::Plan,
            Profile::Careful => Mode::Ask,
            Profile::Standard => Mode::Edit,
            Profile::Trusted => Mode::Auto,
        }
    }

    /// The profile a mode amounts to.
    pub fn of_mode(mode: Mode) -> Profile {
        match mode {
            Mode::Plan => Profile::Locked,
            Mode::Ask => Profile::Careful,
            Mode::Edit => Profile::Standard,
            Mode::Auto => Profile::Trusted,
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Profile::Locked => "reads and plans, changes nothing",
            Profile::Careful => "asks before every edit and command",
            Profile::Standard => "accepts edits, asks before commands",
            Profile::Trusted => "accepts everything without asking",
        }
    }

    /// `self`, or `cap` when `self` is more trusting than the cap allows.
    pub fn capped_by(self, cap: Option<Profile>) -> Profile {
        cap.map_or(self, |c| self.min(c))
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownProfile(pub String);

impl fmt::Display for UnknownProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not a profile; use locked, careful, standard or trusted", self.0)
    }
}

impl FromStr for Profile {
    type Err = UnknownProfile;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Profile::ALL.into_iter().find(|p| p.name().eq_ignore_ascii_case(s.trim())).ok_or_else(|| UnknownProfile(s.trim().to_string()))
    }
}

/// `/profile` with no argument: the profiles, the current one marked.
pub fn list_text(current: Profile, cap: Option<Profile>) -> String {
    let mut lines = vec!["  Permission profiles (most to least restrictive)".to_string()];
    for p in Profile::ALL {
        let here = if p == current { "›" } else { " " };
        let blocked = if cap.is_some_and(|c| p > c) { "  (above this repo's limit)" } else { "" };
        lines.push(format!("  {here} {:<9} {}{blocked}", p.name(), p.about()));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_ordered_and_map_to_modes_both_ways() {
        assert!(Profile::Locked < Profile::Careful && Profile::Careful < Profile::Standard && Profile::Standard < Profile::Trusted);
        for p in Profile::ALL {
            assert_eq!(Profile::of_mode(p.mode()), p);
        }
    }

    #[test]
    fn parsing_ignores_case_and_names_the_choices_on_error() {
        assert_eq!("Standard".parse(), Ok(Profile::Standard));
        assert_eq!(" trusted ".parse(), Ok(Profile::Trusted));
        let e = "yolo".parse::<Profile>().unwrap_err();
        assert!(e.to_string().contains("locked, careful, standard or trusted"), "{e}");
    }

    #[test]
    fn a_cap_lowers_a_profile_and_never_raises_it() {
        assert_eq!(Profile::Trusted.capped_by(Some(Profile::Standard)), Profile::Standard);
        assert_eq!(Profile::Careful.capped_by(Some(Profile::Standard)), Profile::Careful);
        assert_eq!(Profile::Trusted.capped_by(None), Profile::Trusted);
    }

    #[test]
    fn the_list_marks_the_current_profile_and_the_ones_above_the_cap() {
        let text = list_text(Profile::Standard, Some(Profile::Standard));
        assert!(text.contains("› standard"), "{text}");
        assert!(text.contains("trusted   accepts everything without asking  (above this repo's limit)"), "{text}");
        assert!(!text.contains("careful   asks before every edit and command  ("), "{text}");
    }

}
