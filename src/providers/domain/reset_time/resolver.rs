//! Zone-name rules used by reset-time parsing and the system clock adapter.

use super::ZoneExists;

/// An IANA-style zone name: no leading slash or dash, nothing but name characters.
pub(crate) fn is_safe_zone(zone: &str) -> bool {
    !zone.is_empty() && !zone.starts_with(['/', '-']) && zone.bytes().all(|b| b.is_ascii_alphanumeric() || b"_/+-".contains(&b))
}

/// Whether `zone` may be handed to `date`: exactly UTC or GMT, or a real IANA zone.
pub(crate) fn zone_is_known(zone: &str, zone_exists: ZoneExists) -> bool {
    matches!(zone, "UTC" | "GMT") || zone_exists(zone)
}
