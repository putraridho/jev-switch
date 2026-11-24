//! Meters and readings: what one source says about one provider (DESIGN 4.2).

use crate::label::Label;
use crate::quantity::{Currency, Money, Percent};
use crate::time::{UnixMillis, UnixSeconds};
use serde::de::{Error as DeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Which usage window a meter measures.
#[derive(Clone, Eq, PartialEq, Hash, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowKind {
    /// A 5-hour window (300 minutes).
    FiveHour,
    /// A 7-day window (10 080 minutes).
    SevenDay,
    /// A 30-day window (43 200 minutes).
    Monthly,
    /// A rolling window of another length.
    Rolling {
        /// The window's length in minutes.
        minutes: u32,
    },
    /// A window the provider names and whose length is not known.
    Named {
        /// The provider's name for the window.
        #[cfg_attr(feature = "bindings", ts(type = "string"))]
        label: Label,
    },
}

impl WindowKind {
    /// 300, 10 080 and 43 200 minutes map to the named kinds; anything else is `Rolling`
    /// (a length beyond `u32` saturates).
    pub fn from_minutes(m: u64) -> Self {
        match m {
            300 => Self::FiveHour,
            10_080 => Self::SevenDay,
            43_200 => Self::Monthly,
            other => Self::Rolling {
                minutes: u32::try_from(other).unwrap_or(u32::MAX),
            },
        }
    }
}

/// The identity of a meter within one provider.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum MeterId {
    /// A usage window.
    Window(WindowKind),
    /// A balance in one currency.
    Balance(Currency),
}

impl MeterId {
    /// Stable storage/IPC key: `w:5h` `w:7d` `w:30d` `w:r:<minutes>` `w:n:<label>` `b:usd`
    /// `b:cny` `b:other`.
    pub fn key(&self) -> String {
        match self {
            Self::Window(WindowKind::FiveHour) => "w:5h".to_owned(),
            Self::Window(WindowKind::SevenDay) => "w:7d".to_owned(),
            Self::Window(WindowKind::Monthly) => "w:30d".to_owned(),
            Self::Window(WindowKind::Rolling { minutes }) => format!("w:r:{minutes}"),
            Self::Window(WindowKind::Named { label }) => format!("w:n:{}", label.as_str()),
            Self::Balance(Currency::Usd) => "b:usd".to_owned(),
            Self::Balance(Currency::Cny) => "b:cny".to_owned(),
            Self::Balance(Currency::Other) => "b:other".to_owned(),
        }
    }

    /// The exact inverse of [`MeterId::key`]: `Some` only for a string `key` can produce.
    pub fn parse_key(s: &str) -> Option<Self> {
        match s {
            "w:5h" => return Some(Self::Window(WindowKind::FiveHour)),
            "w:7d" => return Some(Self::Window(WindowKind::SevenDay)),
            "w:30d" => return Some(Self::Window(WindowKind::Monthly)),
            "b:usd" => return Some(Self::Balance(Currency::Usd)),
            "b:cny" => return Some(Self::Balance(Currency::Cny)),
            "b:other" => return Some(Self::Balance(Currency::Other)),
            _ => {}
        }
        if let Some(digits) = s.strip_prefix("w:r:") {
            let minutes: u32 = digits.parse().ok()?;
            // Canonical decimal only ("0300" and "+300" are not keys).
            return (minutes.to_string() == digits)
                .then_some(Self::Window(WindowKind::Rolling { minutes }));
        }
        let label = Label::parse(s.strip_prefix("w:n:")?)?;
        Some(Self::Window(WindowKind::Named { label }))
    }
}

impl Serialize for MeterId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.key())
    }
}

impl<'de> Deserialize<'de> for MeterId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyVisitor;
        impl Visitor<'_> for KeyVisitor {
            type Value = MeterId;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a meter key")
            }
            fn visit_str<E: DeError>(self, v: &str) -> Result<MeterId, E> {
                MeterId::parse_key(v).ok_or_else(|| E::custom("not a meter key"))
            }
        }
        deserializer.deserialize_str(KeyVisitor)
    }
}

/// The provider's own verdict on a window.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LimitStatus {
    /// Requests are allowed.
    Allowed,
    /// Allowed, with the provider's warning that the limit is near.
    Warning,
    /// The limit is reached; requests are refused.
    Rejected,
    /// The provider did not say, or said something this version does not know.
    Unknown,
}

/// One usage window as read.
#[derive(Clone, PartialEq, Debug)]
pub struct WindowReading {
    /// Which window.
    pub kind: WindowKind,
    /// How much of it is used.
    pub used: Percent,
    /// When it resets, if the provider says.
    pub resets_at: Option<UnixSeconds>,
    /// The provider's verdict.
    pub status: LimitStatus,
}

/// One balance as read.
#[derive(Clone, PartialEq, Debug)]
pub struct BalanceReading {
    /// The currency.
    pub currency: Currency,
    /// The whole balance.
    pub total: Money,
    /// The granted (free) part, if the provider splits it.
    pub granted: Option<Money>,
    /// The paid part, if the provider splits it.
    pub topped_up: Option<Money>,
    /// Whether the balance can be spent now.
    pub usable: bool,
}

/// One meter of a reading.
#[derive(Clone, PartialEq, Debug)]
pub enum Meter {
    /// A usage window.
    Window(WindowReading),
    /// A balance.
    Balance(BalanceReading),
}

impl Meter {
    /// The meter's identity.
    pub fn id(&self) -> MeterId {
        match self {
            Self::Window(w) => MeterId::Window(w.kind.clone()),
            Self::Balance(b) => MeterId::Balance(b.currency),
        }
    }
}

/// 1..=8 meters with unique ids. The field is private: an empty or duplicate set cannot exist.
#[derive(Clone, PartialEq, Debug)]
pub struct Meters(Vec<Meter>);

impl Meters {
    /// The most meters one reading holds.
    pub const MAX: usize = 8;

    /// `None` for an empty list, more than eight meters, or two meters with the same id.
    pub fn new(v: Vec<Meter>) -> Option<Self> {
        if v.is_empty() || v.len() > Self::MAX {
            return None;
        }
        let mut seen: Vec<MeterId> = Vec::with_capacity(v.len());
        for meter in &v {
            let id = meter.id();
            if seen.contains(&id) {
                return None;
            }
            seen.push(id);
        }
        Some(Self(v))
    }

    /// The meters, in the order given.
    pub fn as_slice(&self) -> &[Meter] {
        &self.0
    }
}

/// What one source learned at one instant.
#[derive(Clone, PartialEq, Debug)]
pub struct Reading {
    /// The meters.
    pub meters: Meters,
    /// The plan name, when the source knows it.
    pub plan: Option<Label>,
    /// When the provider's numbers were observed.
    pub observed_at: UnixMillis,
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

    fn window(kind: WindowKind) -> Meter {
        Meter::Window(WindowReading {
            kind,
            used: Percent::ZERO,
            resets_at: None,
            status: LimitStatus::Allowed,
        })
    }

    fn balance(currency: Currency) -> Meter {
        Meter::Balance(BalanceReading {
            currency,
            total: Money::parse_decimal("1").unwrap(),
            granted: None,
            topped_up: None,
            usable: true,
        })
    }

    fn named(s: &str) -> WindowKind {
        WindowKind::Named {
            label: Label::parse(s).unwrap(),
        }
    }

    #[test]
    fn from_minutes_maps_the_three_known_lengths() {
        assert_eq!(WindowKind::from_minutes(300), WindowKind::FiveHour);
        assert_eq!(WindowKind::from_minutes(10_080), WindowKind::SevenDay);
        assert_eq!(WindowKind::from_minutes(43_200), WindowKind::Monthly);
        assert_eq!(
            WindowKind::from_minutes(0),
            WindowKind::Rolling { minutes: 0 }
        );
        assert_eq!(
            WindowKind::from_minutes(299),
            WindowKind::Rolling { minutes: 299 }
        );
        assert_eq!(
            WindowKind::from_minutes(301),
            WindowKind::Rolling { minutes: 301 }
        );
        assert_eq!(
            WindowKind::from_minutes(1_440),
            WindowKind::Rolling { minutes: 1_440 }
        );
        assert_eq!(
            WindowKind::from_minutes(u64::from(u32::MAX)),
            WindowKind::Rolling { minutes: u32::MAX }
        );
        assert_eq!(
            WindowKind::from_minutes(u64::MAX),
            WindowKind::Rolling { minutes: u32::MAX }
        );
    }

    #[test]
    fn keys_are_the_documented_strings() {
        let cases = [
            (MeterId::Window(WindowKind::FiveHour), "w:5h"),
            (MeterId::Window(WindowKind::SevenDay), "w:7d"),
            (MeterId::Window(WindowKind::Monthly), "w:30d"),
            (
                MeterId::Window(WindowKind::Rolling { minutes: 1_440 }),
                "w:r:1440",
            ),
            (MeterId::Window(named("7d_oi")), "w:n:7d_oi"),
            (MeterId::Balance(Currency::Usd), "b:usd"),
            (MeterId::Balance(Currency::Cny), "b:cny"),
            (MeterId::Balance(Currency::Other), "b:other"),
        ];
        for (id, key) in cases {
            assert_eq!(id.key(), key);
            assert_eq!(MeterId::parse_key(key), Some(id));
        }
    }

    #[test]
    fn parse_key_refuses_everything_key_cannot_produce() {
        for bad in [
            "",
            "w",
            "w:",
            "w:5",
            "w:5H",
            "W:5h",
            " w:5h",
            "w:5h ",
            "w:r:",
            "w:r:-1",
            "w:r:+5",
            "w:r:05",
            "w:r:1.5",
            "w:r:4294967296",
            "w:r:abc",
            "w:n:",
            "w:n:Bad",
            "w:n:a b",
            "w:n:a/b",
            "b:",
            "b:eur",
            "b:USD",
            "x:usd",
            "w:n:jevcanary@example.invalid",
            "w:1h",
        ] {
            assert_eq!(MeterId::parse_key(bad), None, "{bad:?}");
        }
        assert!(MeterId::parse_key(&format!("w:n:{}", "a".repeat(41))).is_none());
    }

    #[test]
    fn meter_id_serialises_as_its_key_and_validates_on_the_way_in() {
        let id = MeterId::Window(named("7d_oi"));
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"w:n:7d_oi\"");
        assert_eq!(
            serde_json::from_str::<MeterId>("\"w:n:7d_oi\"").unwrap(),
            id
        );
        for bad in [
            "\"w:n:NOPE\"",
            "\"\"",
            "\"w:r:05\"",
            "5",
            "null",
            "{\"kind\":\"five_hour\"}",
        ] {
            assert!(serde_json::from_str::<MeterId>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn window_kind_is_internally_tagged() {
        assert_eq!(
            serde_json::to_string(&WindowKind::FiveHour).unwrap(),
            r#"{"kind":"five_hour"}"#
        );
        assert_eq!(
            serde_json::to_string(&WindowKind::SevenDay).unwrap(),
            r#"{"kind":"seven_day"}"#
        );
        assert_eq!(
            serde_json::to_string(&WindowKind::Monthly).unwrap(),
            r#"{"kind":"monthly"}"#
        );
        assert_eq!(
            serde_json::to_string(&WindowKind::Rolling { minutes: 90 }).unwrap(),
            r#"{"kind":"rolling","minutes":90}"#
        );
        assert_eq!(
            serde_json::to_string(&named("7d_oi")).unwrap(),
            r#"{"kind":"named","label":"7d_oi"}"#
        );
    }

    #[test]
    fn limit_status_names() {
        let names: Vec<String> = [
            LimitStatus::Allowed,
            LimitStatus::Warning,
            LimitStatus::Rejected,
            LimitStatus::Unknown,
        ]
        .iter()
        .map(|s| serde_json::to_string(s).unwrap())
        .collect();
        assert_eq!(
            names,
            ["\"allowed\"", "\"warning\"", "\"rejected\"", "\"unknown\""]
        );
    }

    #[test]
    fn meter_id_follows_the_reading() {
        assert_eq!(
            window(WindowKind::SevenDay).id(),
            MeterId::Window(WindowKind::SevenDay)
        );
        assert_eq!(balance(Currency::Cny).id(), MeterId::Balance(Currency::Cny));
    }

    #[test]
    fn meters_rejects_empty() {
        assert_eq!(Meters::new(vec![]), None);
    }

    #[test]
    fn meters_accepts_one_to_eight() {
        let all: Vec<Meter> = (0..8)
            .map(|m| window(WindowKind::Rolling { minutes: m }))
            .collect();
        for n in 1..=8 {
            let meters = Meters::new(all[..n].to_vec()).unwrap();
            assert_eq!(meters.as_slice(), &all[..n]);
        }
    }

    #[test]
    fn meters_rejects_more_than_eight() {
        let nine: Vec<Meter> = (0..9)
            .map(|m| window(WindowKind::Rolling { minutes: m }))
            .collect();
        assert_eq!(Meters::new(nine), None);
    }

    #[test]
    fn meters_rejects_duplicate_ids() {
        assert_eq!(
            Meters::new(vec![
                window(WindowKind::FiveHour),
                window(WindowKind::FiveHour)
            ]),
            None
        );
        assert_eq!(
            Meters::new(vec![balance(Currency::Other), balance(Currency::Other)]),
            None
        );
        assert_eq!(
            Meters::new(vec![
                window(named("a")),
                window(WindowKind::SevenDay),
                window(named("a"))
            ]),
            None
        );
        // Same id, different numbers: still a duplicate.
        let mut second = WindowReading {
            kind: WindowKind::FiveHour,
            used: Percent::FULL,
            resets_at: Some(UnixSeconds(1)),
            status: LimitStatus::Rejected,
        };
        assert_eq!(
            Meters::new(vec![
                window(WindowKind::FiveHour),
                Meter::Window(second.clone())
            ]),
            None
        );
        second.kind = WindowKind::SevenDay;
        assert!(Meters::new(vec![window(WindowKind::FiveHour), Meter::Window(second)]).is_some());
    }

    #[test]
    fn meters_keeps_order_and_mixes_kinds() {
        let v = vec![
            window(WindowKind::FiveHour),
            balance(Currency::Usd),
            window(WindowKind::SevenDay),
        ];
        assert_eq!(Meters::new(v.clone()).unwrap().as_slice(), v.as_slice());
    }

    fn any_window_kind() -> impl Strategy<Value = WindowKind> {
        prop_oneof![
            Just(WindowKind::FiveHour),
            Just(WindowKind::SevenDay),
            Just(WindowKind::Monthly),
            any::<u32>().prop_map(|minutes| WindowKind::Rolling { minutes }),
            "[a-z0-9_]{1,40}".prop_map(|s| WindowKind::Named {
                label: Label::parse(&s).unwrap()
            }),
        ]
    }

    fn any_meter_id() -> impl Strategy<Value = MeterId> {
        prop_oneof![
            any_window_kind().prop_map(MeterId::Window),
            prop_oneof![
                Just(Currency::Usd),
                Just(Currency::Cny),
                Just(Currency::Other)
            ]
            .prop_map(MeterId::Balance),
        ]
    }

    proptest! {
        #[test]
        fn key_round_trips_for_arbitrary_ids(id in any_meter_id()) {
            let key = id.key();
            prop_assert!(key.len() <= 48);
            prop_assert_eq!(MeterId::parse_key(&key), Some(id.clone()));
            let json = serde_json::to_string(&id).unwrap();
            prop_assert_eq!(serde_json::from_str::<MeterId>(&json).unwrap(), id);
        }

        #[test]
        fn parse_key_is_the_exact_inverse(s in "\\PC{0,24}") {
            if let Some(id) = MeterId::parse_key(&s) {
                prop_assert_eq!(id.key(), s);
            }
        }

        #[test]
        fn parse_key_near_misses_never_panic(s in "[wb]:[rn5730dusdcnyother]{0,3}:?[a-z0-9_+. -]{0,44}") {
            if let Some(id) = MeterId::parse_key(&s) {
                prop_assert_eq!(id.key(), s);
            }
        }

        #[test]
        fn from_minutes_is_total(m in any::<u64>()) {
            let kind = WindowKind::from_minutes(m);
            let known = matches!(m, 300 | 10_080 | 43_200);
            prop_assert_eq!(matches!(kind, WindowKind::Rolling { .. }), !known);
        }

        #[test]
        fn meters_new_holds_its_invariant(ids in proptest::collection::vec(any_meter_id(), 0..12)) {
            let v: Vec<Meter> = ids.iter().map(|id| match id {
                MeterId::Window(k) => window(k.clone()),
                MeterId::Balance(c) => balance(*c),
            }).collect();
            let mut unique = ids.clone();
            unique.dedup_by(|a, b| a == b);
            let distinct = ids.iter().enumerate().all(|(i, a)| ids[..i].iter().all(|b| a != b));
            let expected = !ids.is_empty() && ids.len() <= 8 && distinct;
            prop_assert_eq!(Meters::new(v).is_some(), expected);
        }
    }
}
