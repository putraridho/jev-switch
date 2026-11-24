//! The only free-form string of the domain (DESIGN 4.1).

use serde::de::{Error as DeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// The ONLY free-form string in the domain. 1..=40 bytes of `[a-z0-9_]`. Used for plan names
/// ("`max_5x`", "free") and unknown window names ("`7d_oi`"). It cannot hold a prompt, an
/// e-mail, a path or a URL (no space, `@`, `/`, `.` or upper case). It CAN hold 40
/// characters of lower-case identifier, so the rule that protects identity is positional:
/// a `Label` is only ever read from a field that names a plan or a window (DESIGN 8.3).
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct Label(Box<str>);

impl Label {
    /// The longest label, in bytes.
    pub const MAX_LEN: usize = 40;

    /// `Some` when `s` is 1..=40 bytes of `[a-z0-9_]`; `None` otherwise. Nothing is trimmed
    /// or case-folded here: the caller normalises a field it has decided to read.
    pub fn parse(s: &str) -> Option<Self> {
        let valid = !s.is_empty()
            && s.len() <= Self::MAX_LEN
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        valid.then(|| Self(Box::from(s)))
    }

    /// The label's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for Label {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Label {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct LabelVisitor;
        impl Visitor<'_> for LabelVisitor {
            type Value = Label;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("1 to 40 characters of a-z, 0-9 and _")
            }
            fn visit_str<E: DeError>(self, v: &str) -> Result<Label, E> {
                Label::parse(v).ok_or_else(|| E::custom("not a label"))
            }
        }
        deserializer.deserialize_str(LabelVisitor)
    }
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
    use proptest::prelude::*;

    #[test]
    fn accepts_plan_and_window_names() {
        for good in [
            "free",
            "max_5x",
            "7d_oi",
            "a",
            "0",
            "_",
            "plus",
            "primary",
            &"x".repeat(40),
        ] {
            assert_eq!(Label::parse(good).unwrap().as_str(), good);
        }
    }

    #[test]
    fn rejects_everything_outside_the_charset() {
        for bad in [
            "",
            "Free",
            "MAX",
            "max 5x",
            "max-5x",
            "a.b",
            "a/b",
            "a@b",
            "a:b",
            "a\n",
            "\t",
            " a",
            "a ",
            "caf\u{e9}",
            "\u{ff41}",
            "a\0b",
            "user@example.invalid",
            "/home/x",
            "https://x",
            "a+b",
            "{}",
            "\"",
        ] {
            assert_eq!(Label::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn length_is_1_to_40_bytes() {
        assert!(Label::parse("").is_none());
        assert!(Label::parse("a").is_some());
        assert!(Label::parse(&"a".repeat(40)).is_some());
        assert!(Label::parse(&"a".repeat(41)).is_none());
        assert!(Label::parse(&"a".repeat(4096)).is_none());
    }

    #[test]
    fn upper_case_canary_cannot_be_a_label() {
        assert!(Label::parse("JEVCANARY_9f3c1b7e_DO_NOT_SURFACE").is_none());
        // The label-shaped canary can: the protection for it is positional (DESIGN 8.3).
        assert!(Label::parse("jevcanary_9f3c1b7e").is_some());
    }

    #[test]
    fn serde_is_the_plain_string_and_validates_on_the_way_in() {
        let label = Label::parse("max_5x").unwrap();
        assert_eq!(serde_json::to_string(&label).unwrap(), "\"max_5x\"");
        assert_eq!(serde_json::from_str::<Label>("\"max_5x\"").unwrap(), label);
        for bad in [
            "\"Max\"",
            "\"\"",
            "\"a b\"",
            "5",
            "null",
            "[\"a\"]",
            "{\"a\":1}",
        ] {
            assert!(serde_json::from_str::<Label>(bad).is_err(), "{bad}");
        }
        let long = format!("\"{}\"", "a".repeat(41));
        assert!(serde_json::from_str::<Label>(&long).is_err());
    }

    #[test]
    fn debug_shows_only_the_text() {
        assert_eq!(
            format!("{:?}", Label::parse("free").unwrap()),
            "Label(\"free\")"
        );
    }

    proptest! {
        #[test]
        fn parse_agrees_with_the_rule(s in "\\PC{0,48}") {
            let expected = !s.is_empty() && s.len() <= 40
                && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            prop_assert_eq!(Label::parse(&s).is_some(), expected);
        }

        #[test]
        fn valid_labels_round_trip(s in "[a-z0-9_]{1,40}") {
            let label = Label::parse(&s).unwrap();
            prop_assert_eq!(label.as_str(), s.as_str());
            let json = serde_json::to_string(&label).unwrap();
            prop_assert_eq!(serde_json::from_str::<Label>(&json).unwrap(), label);
        }
    }
}
