//! System clock and zone database adapters for reset-time parsing.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::providers::domain::reset_time::resolver::{is_safe_zone, zone_is_known};
use crate::providers::domain::reset_time::wall_clock::is_safe_text;

pub fn system_zone_exists(zone: &str) -> bool {
    zone_in_dir(Path::new("/usr/share/zoneinfo"), zone)
}

pub(crate) fn zone_in_dir(root: &Path, zone: &str) -> bool {
    if !is_safe_zone(zone) {
        return false;
    }
    let (Ok(root), Ok(file)) = (root.canonicalize(), root.join(zone).canonicalize()) else { return false };
    let mut magic = [0u8; 4];
    file.starts_with(&root)
        && std::fs::File::open(&file).and_then(|mut file| file.read_exact(&mut magic)).is_ok()
        && &magic == b"TZif"
}

pub fn system_date(text: &str, zone: Option<&str>) -> Option<u64> {
    if !is_safe_text(text) || text.contains(['(', ')']) || !zone.is_none_or(|zone| is_safe_zone(zone) && zone_is_known(zone, &system_zone_exists)) {
        return None;
    }
    let mut cmd = Command::new("date");
    cmd.args(["-d", text, "+%s"]).env("LC_ALL", "C").stdin(Stdio::null()).stderr(Stdio::null());
    if let Some(zone) = zone {
        cmd.env("TZ", zone);
    }
    let out = cmd.output().ok().filter(|output| output.status.success())?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::domain::reset_time::wall_clock::{normalize_for_date, split_zone};

    #[cfg(unix)]
    #[test]
    fn a_zone_must_be_a_tzif_file_inside_the_zone_directory() {
        let dir = crate::testkit::scratch_dir::scratch_dir("cooldown", "zones");
        let root = dir.join("zoneinfo");
        std::fs::create_dir_all(root.join("Asia")).unwrap();
        std::fs::write(root.join("Asia/Jakarta"), b"TZif2 rest of the file").unwrap();
        std::fs::write(root.join("zone.tab"), b"# not a zone").unwrap();
        std::fs::write(dir.join("outside"), b"TZif2 but outside").unwrap();
        std::os::unix::fs::symlink(dir.join("outside"), root.join("Escape")).unwrap();
        std::os::unix::fs::symlink(root.join("Asia/Jakarta"), root.join("Alias")).unwrap();
        for ok in ["Asia/Jakarta", "Alias"] {
            assert!(zone_in_dir(&root, ok), "{ok}");
        }
        for bad in ["PST", "Not/AZone", "Asia", "zone.tab", "Escape", "../outside", "/etc/passwd", "", "Asia/../zone.tab"] {
            assert!(!zone_in_dir(&root, bad), "{bad}");
        }
        assert!(!zone_in_dir(&dir.join("missing"), "Asia/Jakarta"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_real_resolver_refuses_abbreviations_and_unknown_zones() {
        for zone in ["PST", "Not/AZone", "../etc", "/etc/passwd", ""] {
            assert_eq!(system_date("5pm", Some(zone)), None);
        }
        let long = "a".repeat(500);
        for text in ["$(touch /tmp/pwned)", "`id`", "5pm; rm -rf /", "5pm && ls", "5pm | cat", "@1760000000", "\"5pm\"", "5pm\n6pm", "5pm (Asia/Jakarta; ls)", "5pm (../../etc/passwd)", "5pm (/etc/passwd)", "5pm (-x)", "5pm ()", "(Asia/Jakarta)", "5pm (Asia/Jakarta", "5pm )", "5pm (a) (b", "17 h\u{e9}", "\u{0}", &long] {
            assert_eq!(system_date(text, None), None, "{text:?}");
        }
    }

    #[test]
    fn the_real_resolver_runs_date_when_it_exists() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let Some(at) = system_date("1 hour", None) else { return };
        assert!((now + 3590..=now + 3700).contains(&at), "{at} vs {now}");
        for text in ["5pm (Asia/Jakarta)", "Nov 20, 5pm (Asia/Jakarta)", "Nov 20 at 5:30pm (UTC)"] {
            let (body, zone) = split_zone(text).unwrap();
            if zone.is_some_and(|zone| !zone_is_known(zone, &system_zone_exists)) { continue; }
            assert!(system_date(&normalize_for_date(body), zone).is_some(), "{text}");
        }
    }
}
