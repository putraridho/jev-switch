//! The state of one provider (DESIGN 4.3). `Usage` is derived, never stored.

use crate::meter::Reading;
use crate::time::UnixMillis;
use serde::Serialize;

/// What the app knows about one provider right now.
#[derive(Clone, PartialEq, Debug)]
pub enum Usage {
    /// No source has answered yet in this run.
    Pending,
    /// A reading that is current by the provider's freshness policy.
    Known(Reading),
    /// The last good reading, kept, with the instant it stopped being current and why.
    Stale {
        /// The last good reading.
        last: Reading,
        /// When it stopped being current.
        since: UnixMillis,
        /// Why it is not current.
        cause: StaleCause,
    },
    /// No reading to show.
    Unavailable {
        /// Why there is none.
        reason: Unavailable,
    },
    /// The user has to do something first.
    NotConfigured {
        /// What to do.
        setup: Setup,
    },
    /// No usage source the app can read exists for this provider (placeholders). It says
    /// nothing about whether the user has a plan there: the app cannot know (DESIGN 1a).
    Unsupported,
}

/// Why a kept reading is not current.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StaleCause {
    /// A passive source saw nothing new.
    Quiet,
    /// The source is failing.
    Failing {
        /// The failure.
        reason: Unavailable,
    },
}

/// Why a source could not answer. A closed list: no error carries a string.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Unavailable {
    /// The provider's CLI is not installed.
    NotInstalled,
    /// The provider's CLI is not signed in.
    NotLoggedIn,
    /// The network could not be reached.
    Offline,
    /// The source did not answer in time.
    Timeout,
    /// The provider refused because of its rate limit.
    RateLimited,
    /// The provider refused the credentials.
    Unauthorized,
    /// The provider answered with a server error.
    ServerError,
    /// The answer did not have the expected shape.
    Protocol,
    /// A local file could not be read.
    Io,
    /// A fault inside the app.
    Internal,
}

/// What the user has to do before a provider can show anything.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub enum Setup {
    /// Use Claude through the gateway once, so its log exists.
    #[serde(rename = "use_gateway")]
    UseGateway,
    /// Add `DEEPSEEK_API_KEY` to the env file.
    #[serde(rename = "add_deepseek_key")]
    AddDeepSeekKey,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;

    const ALL_UNAVAILABLE: [(Unavailable, &str); 10] = [
        (Unavailable::NotInstalled, "not_installed"),
        (Unavailable::NotLoggedIn, "not_logged_in"),
        (Unavailable::Offline, "offline"),
        (Unavailable::Timeout, "timeout"),
        (Unavailable::RateLimited, "rate_limited"),
        (Unavailable::Unauthorized, "unauthorized"),
        (Unavailable::ServerError, "server_error"),
        (Unavailable::Protocol, "protocol"),
        (Unavailable::Io, "io"),
        (Unavailable::Internal, "internal"),
    ];

    #[test]
    fn unavailable_names() {
        for (value, name) in ALL_UNAVAILABLE {
            assert_eq!(
                serde_json::to_string(&value).unwrap(),
                format!("\"{name}\"")
            );
        }
    }

    #[test]
    fn setup_has_two_variants_with_fixed_names() {
        assert_eq!(
            serde_json::to_string(&Setup::UseGateway).unwrap(),
            "\"use_gateway\""
        );
        assert_eq!(
            serde_json::to_string(&Setup::AddDeepSeekKey).unwrap(),
            "\"add_deepseek_key\""
        );
    }

    #[test]
    fn stale_cause_is_internally_tagged_with_a_struct_variant() {
        assert_eq!(
            serde_json::to_string(&StaleCause::Quiet).unwrap(),
            r#"{"kind":"quiet"}"#
        );
        for (reason, name) in ALL_UNAVAILABLE {
            assert_eq!(
                serde_json::to_string(&StaleCause::Failing { reason }).unwrap(),
                format!(r#"{{"kind":"failing","reason":"{name}"}}"#)
            );
        }
    }

    #[test]
    fn usage_debug_carries_no_free_text() {
        let text = format!(
            "{:?}",
            Usage::Unavailable {
                reason: Unavailable::Io
            }
        );
        assert_eq!(text, "Unavailable { reason: Io }");
        assert_eq!(
            format!(
                "{:?}",
                Usage::NotConfigured {
                    setup: Setup::UseGateway
                }
            ),
            "NotConfigured { setup: UseGateway }"
        );
        assert_eq!(format!("{:?}", Usage::Pending), "Pending");
        assert_eq!(format!("{:?}", Usage::Unsupported), "Unsupported");
    }
}
