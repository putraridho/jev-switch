//! What one line typed into `jev-code` means. Pure, so the REPL binary only maps
//! each [`Command`] onto the [`crate::turn::Engine`] and every spelling rule is a
//! unit test.

use crate::routing::Pin;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// A blank line.
    Empty,
    /// Anything that is not a command: sent to the model.
    Prompt(String),
    Quit,
    New,
    /// `/model` with no argument: show the current pin.
    ShowModel,
    /// `/model <arg>`.
    SetPin(Pin),
    /// `/provider auto|off`: routing across providers on (true) or off (Claude only).
    SetRouting(bool),
    /// `/orchestrate on|off`: a planner that delegates pieces to workers, for this session.
    SetOrchestrate(bool),
    /// `/orchestrate` with no argument: show whether it is on.
    ShowOrchestrate,
    /// `/providers [refresh]`.
    Providers { refresh: bool },
    /// A command that was recognised but whose argument was not; the text says what is accepted.
    Invalid(String),
}

/// Whether provider routing starts on: `JEV_PROVIDERS=off` (also `0`, `false`, `no`)
/// starts it off, anything else, including unset, leaves the default (on).
pub fn routing_enabled(env: Option<&str>) -> bool {
    !env.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no"))
}

/// Read one input line.
pub fn parse(line: &str) -> Command {
    let line = line.trim();
    let mut words = line.split_whitespace();
    let Some(head) = words.next() else { return Command::Empty };
    let arg = words.next();
    let extra = words.next().is_some();
    let invalid = |usage: &str| Command::Invalid(usage.to_string());
    match head {
        "/quit" | "/exit" => Command::Quit,
        "/new" => Command::New,
        "/model" => match (arg, extra) {
            (None, _) => Command::ShowModel,
            (Some(a), false) => Pin::parse(a).map_or_else(Command::Invalid, Command::SetPin),
            (Some(_), true) => invalid("usage: /model auto|<provider>|<provider>:<model>|sonnet|opus|haiku"),
        },
        "/provider" => match (arg, extra) {
            (Some("auto" | "on"), false) => Command::SetRouting(true),
            (Some("off"), false) => Command::SetRouting(false),
            _ => invalid("usage: /provider auto|off"),
        },
        "/orchestrate" => match (arg, extra) {
            (None, _) => Command::ShowOrchestrate,
            (Some("on"), false) => Command::SetOrchestrate(true),
            (Some("off"), false) => Command::SetOrchestrate(false),
            _ => invalid("usage: /orchestrate on|off"),
        },
        "/providers" => match (arg, extra) {
            (None, _) => Command::Providers { refresh: false },
            (Some("refresh"), false) => Command::Providers { refresh: true },
            _ => invalid("usage: /providers [refresh]"),
        },
        // A typo'd command must not cost a model turn; a path like `/etc/hosts` is still a prompt.
        h if h.starts_with('/') && !h[1..].contains('/') => Command::Invalid(format!("unknown command {h}")),
        _ => Command::Prompt(line.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ProviderId;

    #[test]
    fn plain_text_is_a_prompt_and_blank_is_empty() {
        assert_eq!(parse("fix the bug"), Command::Prompt("fix the bug".into()));
        assert_eq!(parse("   "), Command::Empty);
        assert_eq!(parse("/etc/hosts looks odd"), Command::Prompt("/etc/hosts looks odd".into()));
    }

    #[test]
    fn simple_commands() {
        assert_eq!(parse("/quit"), Command::Quit);
        assert_eq!(parse("/exit"), Command::Quit);
        assert_eq!(parse("/new"), Command::New);
        assert_eq!(parse("/providers"), Command::Providers { refresh: false });
        assert_eq!(parse("/providers refresh"), Command::Providers { refresh: true });
        assert!(matches!(parse("/providers nope"), Command::Invalid(_)));
    }

    #[test]
    fn model_pins() {
        assert_eq!(parse("/model"), Command::ShowModel);
        assert_eq!(parse("/model auto"), Command::SetPin(Pin::Auto));
        assert_eq!(parse("/model codex"), Command::SetPin(Pin::Provider(ProviderId::Codex)));
        assert_eq!(
            parse("/model sonnet"),
            Command::SetPin(Pin::Model(ProviderId::Claude, "claude-sonnet-5-5".into()))
        );
        assert_eq!(
            parse("/model codex:gpt-6.1-sol"),
            Command::SetPin(Pin::Model(ProviderId::Codex, "gpt-6.1-sol".into()))
        );
        assert!(matches!(parse("/model nonsense"), Command::Invalid(m) if m.contains("unknown provider")));
        assert!(matches!(parse("/model codex:nope"), Command::Invalid(m) if m.contains("unknown model")));
        assert!(matches!(parse("/model a b"), Command::Invalid(_)));
    }

    #[test]
    fn routing_is_on_unless_the_env_says_off() {
        assert!(routing_enabled(None));
        assert!(routing_enabled(Some("auto")));
        assert!(routing_enabled(Some("")));
        for off in ["off", "OFF", " 0 ", "false", "no"] {
            assert!(!routing_enabled(Some(off)), "{off}");
        }
    }

    #[test]
    fn pins_print_the_way_they_are_typed() {
        for arg in ["auto", "codex", "claude:claude-opus-5-5"] {
            assert_eq!(Pin::parse(arg).unwrap().to_string(), arg);
        }
    }

    #[test]
    fn routing_toggle() {
        assert_eq!(parse("/provider auto"), Command::SetRouting(true));
        assert_eq!(parse("/provider off"), Command::SetRouting(false));
        assert!(matches!(parse("/provider"), Command::Invalid(_)));
        assert!(matches!(parse("/provider maybe"), Command::Invalid(_)));
    }

    #[test]
    fn orchestrate_toggle() {
        assert_eq!(parse("/orchestrate on"), Command::SetOrchestrate(true));
        assert_eq!(parse("/orchestrate off"), Command::SetOrchestrate(false));
        assert_eq!(parse("/orchestrate"), Command::ShowOrchestrate);
        assert!(matches!(parse("/orchestrate maybe"), Command::Invalid(m) if m.contains("on|off")));
        assert!(matches!(parse("/orchestrate on now"), Command::Invalid(_)));
    }

    #[test]
    fn unknown_slash_command_is_reported_not_sent() {
        assert!(matches!(parse("/provders"), Command::Invalid(m) if m.contains("/provders")));
    }
}
