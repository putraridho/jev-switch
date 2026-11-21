//! The production [`Resolver`](super::Resolver) and [`ZoneExists`](super::ZoneExists):
//! GNU `date` and the system zone directory, plus the zone-name rules both rely on.

use std::path::Path;
use std::process::{Command, Stdio};

use super::wall_clock::is_safe_text;
use super::ZoneExists;

/// Where the system keeps its IANA zone files.
const ZONEINFO_DIR: &str = "/usr/share/zoneinfo";

/// An IANA-style zone name: no leading slash or dash, nothing but name characters.
pub(super) fn is_safe_zone(zone: &str) -> bool {
    !zone.is_empty() && !zone.starts_with(['/', '-']) && zone.bytes().all(|b| b.is_ascii_alphanumeric() || b"_/+-".contains(&b))
}

/// Whether `zone` may be handed to `date`: exactly UTC or GMT, or a real IANA zone.
pub(super) fn zone_is_known(zone: &str, zone_exists: ZoneExists) -> bool {
    matches!(zone, "UTC" | "GMT") || zone_exists(zone)
}

/// The production [`ZoneExists`]: `zone` is a TZif file under the system zone
/// directory. See [`zone_in_dir`].
pub(super) fn system_zone_exists(zone: &str) -> bool {
    zone_in_dir(Path::new(ZONEINFO_DIR), zone)
}

/// Whether `zone` names a real zone file under `root`: a safe name, a file that,
/// symlinks followed, is still inside `root`, and that starts with the TZif magic (so
/// `zone.tab` and friends are not zones). Without this check `date` reads an unknown
/// `TZ` such as `PST` as UTC and exits 0.
pub(super) fn zone_in_dir(root: &Path, zone: &str) -> bool {
    use std::io::Read;
    if !is_safe_zone(zone) {
        return false;
    }
    let (Ok(root), Ok(file)) = (root.canonicalize(), root.join(zone).canonicalize()) else { return false };
    let mut magic = [0u8; 4];
    file.starts_with(&root)
        && std::fs::File::open(&file).and_then(|mut f| f.read_exact(&mut magic)).is_ok()
        && &magic == b"TZif"
}

/// The production [`Resolver`]: GNU `date -d <text> +%s`, run directly (never through
/// a shell) with the zone in `TZ`. Re-validates its input, so it is safe on its own.
pub(super) fn system_date(text: &str, zone: Option<&str>) -> Option<u64> {
    if !is_safe_text(text) || text.contains(['(', ')']) || !zone.is_none_or(|z| is_safe_zone(z) && zone_is_known(z, &system_zone_exists)) {
        return None;
    }
    let mut cmd = Command::new("date");
    cmd.args(["-d", text, "+%s"]).env("LC_ALL", "C").stdin(Stdio::null()).stderr(Stdio::null());
    if let Some(z) = zone {
        cmd.env("TZ", z);
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}
