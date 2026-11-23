//! What `/verify` and `JEV_VERIFY` choose: whether a turn's changes are checked, and with what.

/// How a turn that changed files is checked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VerifyMode {
    /// Never run a check (the default: a check costs time and CPU).
    #[default]
    Off,
    /// Run the project's cheapest check (`cargo check`, `npm test`, `pytest -x`), found from
    /// its manifests, after a turn that changed files.
    Auto,
    /// Run this command in the project folder after a turn that changed files.
    Command(String),
}

impl VerifyMode {
    /// The argument of `/verify` or the value of `JEV_VERIFY`: `off`, `auto`, or a command.
    /// The old switch words keep working: `on` is `auto`.
    pub fn parse(arg: &str) -> Result<Self, String> {
        let arg = arg.trim();
        match arg.to_ascii_lowercase().as_str() {
            "" => Err("give off, auto or a command to run".to_string()),
            "off" | "0" | "false" | "no" => Ok(Self::Off),
            "auto" | "on" | "1" | "true" | "yes" => Ok(Self::Auto),
            _ => Ok(Self::Command(arg.to_string())),
        }
    }

    /// The environment's choice; unset or empty is off.
    pub fn from_env(value: Option<&str>) -> Self {
        value.and_then(|v| Self::parse(v).ok()).unwrap_or_default()
    }

    pub fn is_off(&self) -> bool {
        *self == Self::Off
    }

    /// How the setting reads back to the person.
    pub fn label(&self) -> String {
        match self {
            Self::Off => "off".to_string(),
            Self::Auto => "auto".to_string(),
            Self::Command(c) => format!("command: {c}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_and_a_command_parse() {
        assert_eq!(VerifyMode::parse("off"), Ok(VerifyMode::Off));
        assert_eq!(VerifyMode::parse(" Auto "), Ok(VerifyMode::Auto));
        assert_eq!(VerifyMode::parse("on"), Ok(VerifyMode::Auto));
        assert_eq!(VerifyMode::parse("make lint"), Ok(VerifyMode::Command("make lint".into())));
        assert!(VerifyMode::parse("  ").is_err());
    }

    #[test]
    fn the_environment_defaults_to_off() {
        assert_eq!(VerifyMode::from_env(None), VerifyMode::Off);
        assert_eq!(VerifyMode::from_env(Some("")), VerifyMode::Off);
        assert_eq!(VerifyMode::from_env(Some("auto")), VerifyMode::Auto);
        assert_eq!(VerifyMode::Command("x y".into()).label(), "command: x y");
    }
}
