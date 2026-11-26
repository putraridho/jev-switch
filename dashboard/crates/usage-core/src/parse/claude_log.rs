//! Allow-list for a single gateway usage row.
use crate::label::Label;
use crate::meter::{LimitStatus, Meter, Meters, WindowKind, WindowReading};
use crate::quantity::Percent;
use crate::time::{UnixMillis, UnixSeconds};
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;

/// The usage windows of one log row.
#[derive(Clone, PartialEq, Debug)]
pub struct ClaudeLogSample {
    /// The row's timestamp.
    pub at: UnixMillis,
    /// The row's windows.
    pub windows: Meters,
}
#[derive(Default)]
struct Parts {
    used: Option<Percent>,
    reset: Option<UnixSeconds>,
    status: Option<LimitStatus>,
}
struct Windows(Vec<Meter>);
impl<'de> Deserialize<'de> for Windows {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct WindowsVisitor;
        impl<'de> Visitor<'de> for WindowsVisitor {
            type Value = Windows;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("rate limit headers")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Windows, A::Error> {
                let mut parts: Vec<(Label, Parts)> = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    let Some((name, attribute)) = key
                        .strip_prefix("anthropic-ratelimit-unified-")
                        .and_then(|s| s.rsplit_once('-'))
                    else {
                        map.next_value::<IgnoredAny>()?;
                        continue;
                    };
                    if !matches!(attribute, "utilization" | "reset" | "status")
                        || Label::parse(name).is_none()
                    {
                        map.next_value::<IgnoredAny>()?;
                        continue;
                    }
                    let value: Header = map.next_value()?;
                    let Some(label) = Label::parse(name) else {
                        continue;
                    };
                    if !parts.iter().any(|(existing, _)| *existing == label) {
                        parts.push((label.clone(), Parts::default()));
                    }
                    let Some((_, entry)) =
                        parts.iter_mut().find(|(existing, _)| *existing == label)
                    else {
                        continue;
                    };
                    match attribute {
                        "utilization" if entry.used.is_none() => {
                            entry.used = value.used;
                        }
                        "reset" if entry.reset.is_none() => {
                            entry.reset = value.reset;
                        }
                        "status" if entry.status.is_none() => {
                            entry.status = Some(value.status);
                        }
                        _ => {}
                    }
                }
                let mut meters = Vec::new();
                parts.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
                for (name, value) in parts {
                    let Some(used) = value.used else {
                        continue;
                    };
                    let kind = match name.as_str() {
                        "5h" => WindowKind::FiveHour,
                        "7d" => WindowKind::SevenDay,
                        _ => WindowKind::Named { label: name },
                    };
                    meters.push(Meter::Window(WindowReading {
                        kind,
                        used,
                        resets_at: value.reset,
                        status: value.status.unwrap_or(LimitStatus::Unknown),
                    }));
                    if meters.len() == Meters::MAX {
                        break;
                    }
                }
                Ok(Windows(meters))
            }
        }
        deserializer.deserialize_map(WindowsVisitor)
    }
}
struct Header {
    used: Option<Percent>,
    reset: Option<UnixSeconds>,
    status: LimitStatus,
}
impl Header {
    fn empty() -> Self {
        Self {
            used: None,
            reset: None,
            status: LimitStatus::Unknown,
        }
    }
}
impl<'de> Deserialize<'de> for Header {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct HeaderVisitor;
        impl<'de> Visitor<'de> for HeaderVisitor {
            type Value = Header;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a header")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Header, E> {
                Ok(Header {
                    used: v.parse::<f64>().ok().and_then(Percent::from_fraction),
                    reset: v.parse().ok().map(UnixSeconds),
                    status: match v {
                        "allowed" => LimitStatus::Allowed,
                        "allowed_warning" => LimitStatus::Warning,
                        "rejected" => LimitStatus::Rejected,
                        _ => LimitStatus::Unknown,
                    },
                })
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Header, E> {
                Ok(Header::empty())
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Header, E> {
                Ok(Header::empty())
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Header, E> {
                Ok(Header::empty())
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Header, E> {
                Ok(Header::empty())
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Header, E> {
                Ok(Header::empty())
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Header, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Header::empty())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Header, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Header::empty())
            }
        }
        d.deserialize_any(HeaderVisitor)
    }
}
#[derive(Deserialize)]
struct Row {
    ts_ms: u64,
    ratelimit: Windows,
}
/// Cheap byte prefilter for compact gateway rows.
pub fn might_have_sample(line: &[u8]) -> bool {
    line.windows(b"\"ratelimit\":{\"".len())
        .any(|part| part == b"\"ratelimit\":{\"")
}
/// Parse a single compact row.
pub fn parse_line(line: &[u8]) -> Option<ClaudeLogSample> {
    if !might_have_sample(line) {
        return None;
    }
    let row: Row = serde_json::from_slice(line).ok()?;
    Some(ClaudeLogSample {
        at: UnixMillis(row.ts_ms),
        windows: Meters::new(row.ratelimit.0)?,
    })
}
