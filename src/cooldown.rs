//! Remembering which providers hit their usage limit, so the router skips them
//! until they reset instead of paying a failed turn to rediscover it.
//!
//! Time is always passed in (`now_ms`), wall-clock text is resolved through an
//! injected [`Resolver`], zone names are checked through an injected [`ZoneExists`],
//! and the file path is always a parameter, so every rule here is testable without a
//! clock, `date`, the zone database, or the real state directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use serde_json::{Map, Value};

use crate::backend::ProviderId;

const SECOND_MS: u64 = 1000;
const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const DAY_MS: u64 = 24 * HOUR_MS;
/// Used when the CLI gave no reset time we can read. An hour is long enough not to
/// hammer a limited provider and short enough to recover the same day.
const DEFAULT_MS: u64 = HOUR_MS;
/// Never trust a parsed reset to be shorter than this (clock skew, "resets now").
const MIN_MS: u64 = MINUTE_MS;
/// Never wait longer than the longest plan window (weekly limits), so one bad
/// timestamp cannot disable a provider for months.
const MAX_MS: u64 = 8 * DAY_MS;
/// Epoch values at or above this are milliseconds; below it, seconds. 1e11 s is the
/// year 5138 and 1e11 ms is 1973, so real timestamps never straddle the line.
const MS_THRESHOLD: u64 = 100_000_000_000;
/// Reset text longer than this is not a reset time; it is never handed to `date`.
const MAX_TEXT_LEN: usize = 64;

/// Turns wall-clock text (and an optional IANA zone) into epoch seconds, or `None`.
/// Injected so tests never run `date`; production passes [`system_date`].
pub type Resolver<'a> = &'a dyn Fn(&str, Option<&str>) -> Option<u64>;

/// Whether a zone name is a real IANA zone. Injected so tests need no zone database;
/// production passes [`system_zone_exists`].
pub type ZoneExists<'a> = &'a dyn Fn(&str) -> bool;

/// Where the system keeps its IANA zone files.
const ZONEINFO_DIR: &str = "/usr/share/zoneinfo";

/// Milliseconds a provider should be skipped, given the CLI's reset text, resolving
/// wall-clock forms with the system `date`. See [`cooldown_ms_in`].
pub fn cooldown_ms(resets_at: Option<&str>, now_ms: u64) -> u64 {
    cooldown_ms_with(resets_at, now_ms, &system_date)
}

/// Milliseconds a provider should be skipped, given the CLI's reset text.
///
/// Understood forms: epoch seconds or milliseconds as digits; a duration
/// ("3h20m5s", "20m", "90 minutes", "2 hours"); a time of day or date and time
/// ("5pm (Asia/Jakarta)", "Nov 20, 5pm (Asia/Jakarta)") and ISO-8601, both resolved
/// by `resolve`. Zones are checked against the system zone database. See
/// [`cooldown_ms_in`] for the rules.
pub fn cooldown_ms_with(resets_at: Option<&str>, now_ms: u64, resolve: Resolver) -> u64 {
    cooldown_ms_in(resets_at, now_ms, resolve, &system_zone_exists)
}

/// [`cooldown_ms_with`] with an injected zone check: a zone is accepted only if it is
/// exactly `UTC`/`GMT` or `zone_exists` says it is a real IANA zone. An abbreviation
/// such as `PST` is ambiguous (and `date` would silently read it as UTC), so it is as
/// unreadable as any other unknown zone.
///
/// A bare time of day that is not in the future means tomorrow. A date with no year
/// that is not in the future (said on Dec 31: "Jan 2 5pm") means next year, if that is
/// within the 8-day cap. Anything unreadable (or nothing) means [`DEFAULT_MS`]; any
/// result is clamped to [1 minute, 8 days].
pub fn cooldown_ms_in(resets_at: Option<&str>, now_ms: u64, resolve: Resolver, zone_exists: ZoneExists) -> u64 {
    let reset_ms = resets_at
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| reset_instant_ms(s, now_ms, resolve, zone_exists));
    let wait = reset_ms.map_or(DEFAULT_MS, |r| r.saturating_sub(now_ms));
    wait.clamp(MIN_MS, MAX_MS)
}

/// The reset instant (epoch ms) the text names, by form: epoch, duration, wall clock.
fn reset_instant_ms(text: &str, now_ms: u64, resolve: Resolver, zone_exists: ZoneExists) -> Option<u64> {
    if text.bytes().all(|b| b.is_ascii_digit()) {
        let n: u64 = text.parse().ok()?;
        return Some(if n >= MS_THRESHOLD { n } else { n.saturating_mul(1000) });
    }
    if let Some(d) = parse_duration_ms(text) {
        return Some(now_ms.saturating_add(d));
    }
    wall_clock_ms(text, now_ms, resolve, zone_exists)
}

/// "3h20m5s", "3h 20m", "20m", "90 minutes", "2 hours", "in 1 day": a sum of
/// number+unit parts and nothing else.
fn parse_duration_ms(text: &str) -> Option<u64> {
    if text.len() > MAX_TEXT_LEN {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    let mut rest = lower.strip_prefix("in ").unwrap_or(&lower).trim_start();
    let mut total = 0u64;
    let mut parts = 0;
    while !rest.is_empty() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let n: u64 = rest[..digits].parse().ok()?;
        rest = rest[digits..].trim_start();
        let letters = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
        let unit_ms = match &rest[..letters] {
            "d" | "day" | "days" => DAY_MS,
            "h" | "hr" | "hrs" | "hour" | "hours" => HOUR_MS,
            "m" | "min" | "mins" | "minute" | "minutes" => MINUTE_MS,
            "s" | "sec" | "secs" | "second" | "seconds" => SECOND_MS,
            _ => return None,
        };
        total = total.saturating_add(n.saturating_mul(unit_ms));
        parts += 1;
        rest = rest[letters..].trim_start_matches(|c: char| c == ',' || c.is_whitespace());
    }
    (parts > 0).then_some(total)
}

/// Text `date -d` may be given: letters, digits, spaces and `, : / _ + - . ( )` only.
/// No quotes, `$`, `@` (epoch syntax), backticks, `;`, `&`, `|`, newlines or non-ASCII.
fn is_safe_text(text: &str) -> bool {
    text.len() <= MAX_TEXT_LEN && text.bytes().all(|b| b.is_ascii_alphanumeric() || b" ,:/_+-.()".contains(&b))
}

/// An IANA-style zone name: no leading slash or dash, nothing but name characters.
fn is_safe_zone(zone: &str) -> bool {
    !zone.is_empty() && !zone.starts_with(['/', '-']) && zone.bytes().all(|b| b.is_ascii_alphanumeric() || b"_/+-".contains(&b))
}

/// `"5pm (Asia/Jakarta)"` into `("5pm", Some("Asia/Jakarta"))`; without a trailing
/// parenthesis the whole text is the body.
fn split_zone(text: &str) -> Option<(&str, Option<&str>)> {
    let Some(inner) = text.strip_suffix(')') else { return (!text.contains(['(', ')'])).then_some((text, None)) };
    let open = inner.rfind('(')?;
    let (body, zone) = (inner[..open].trim_end(), inner[open + 1..].trim());
    (!body.is_empty() && !body.contains(['(', ')']) && is_safe_zone(zone)).then_some((body, Some(zone)))
}

/// "5pm", "5:30 pm", "17:00": a time of day with no date.
fn is_bare_time_of_day(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    let (clock, meridiem) = match lower.strip_suffix("am").or_else(|| lower.strip_suffix("pm")) {
        Some(c) => (c.trim_end(), true),
        None => (lower.as_str(), false),
    };
    let fields: Vec<&str> = clock.split(':').collect();
    let numeric = fields.iter().all(|f| (1..=2).contains(&f.len()) && f.bytes().all(|b| b.is_ascii_digit()));
    numeric && fields.len() <= 3 && (meridiem || fields.len() > 1)
}

/// GNU `date` rejects "Nov 20, 5pm" and "Nov 20 at 5pm" but takes "Nov 20 5pm": drop
/// the commas and the word "at" from spaced text. Text without spaces (ISO-8601) is
/// left alone.
fn normalize_for_date(body: &str) -> String {
    if !body.contains(' ') {
        return body.to_string();
    }
    let spaced = body.replace(',', " ");
    spaced.split_whitespace().filter(|w| !w.eq_ignore_ascii_case("at")).collect::<Vec<_>>().join(" ")
}

const MONTHS: [&str; 24] = [
    "jan", "january", "feb", "february", "mar", "march", "apr", "april", "may", "jun", "june", "jul", "july", "aug",
    "august", "sep", "sept", "september", "oct", "october", "nov", "november", "dec", "december",
];

/// A date that names a month but no four-digit year: "Jan 2 5pm", "November 20".
fn names_date_without_year(body: &str) -> bool {
    let words = || body.split_whitespace().map(|w| w.trim_end_matches('.'));
    let has_year = words().any(|w| w.len() == 4 && w.bytes().all(|b| b.is_ascii_digit()));
    !has_year && words().any(|w| MONTHS.contains(&w.to_ascii_lowercase().as_str()))
}

/// The UTC calendar year of an instant (days since 1970 to a civil year).
fn utc_year(ms: u64) -> u64 {
    let z = (ms / DAY_MS) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month = (5 * doy + 2) / 153; // 0 = March
    (yoe + era * 400 + i64::from(month >= 10)) as u64
}

/// A yearless date that came out in the past, asked again for next year. Taken only
/// if that is in the future and within [`MAX_MS`]: a stale date far ahead is more
/// likely a misread than a real reset.
fn next_year_instant(body: &str, zone: Option<&str>, now_ms: u64, resolve: Resolver) -> Option<u64> {
    let next = resolve(&format!("{body} {}", utc_year(now_ms) + 1), zone)?.saturating_mul(1000);
    (next > now_ms && next - now_ms <= MAX_MS).then_some(next)
}

/// Whether `zone` may be handed to `date`: exactly UTC or GMT, or a real IANA zone.
fn zone_is_known(zone: &str, zone_exists: ZoneExists) -> bool {
    matches!(zone, "UTC" | "GMT") || zone_exists(zone)
}

/// A wall-clock reset (time of day, date and time, ISO-8601) via `resolve`.
fn wall_clock_ms(text: &str, now_ms: u64, resolve: Resolver, zone_exists: ZoneExists) -> Option<u64> {
    if !is_safe_text(text) {
        return None;
    }
    let (body, zone) = split_zone(text)?;
    if zone.is_some_and(|z| !zone_is_known(z, zone_exists)) {
        return None;
    }
    let body = normalize_for_date(body);
    let ms = resolve(&body, zone)?.saturating_mul(1000);
    if ms > now_ms {
        return Some(ms);
    }
    if is_bare_time_of_day(&body) {
        return Some(ms.saturating_add(DAY_MS)); // "resets 5pm" said after 5pm means tomorrow
    }
    if names_date_without_year(&body) {
        return Some(next_year_instant(&body, zone, now_ms, resolve).unwrap_or(ms));
    }
    Some(ms)
}

/// The production [`ZoneExists`]: `zone` is a TZif file under the system zone
/// directory. See [`zone_in_dir`].
pub fn system_zone_exists(zone: &str) -> bool {
    zone_in_dir(Path::new(ZONEINFO_DIR), zone)
}

/// Whether `zone` names a real zone file under `root`: a safe name, a file that,
/// symlinks followed, is still inside `root`, and that starts with the TZif magic (so
/// `zone.tab` and friends are not zones). Without this check `date` reads an unknown
/// `TZ` such as `PST` as UTC and exits 0.
fn zone_in_dir(root: &Path, zone: &str) -> bool {
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
pub fn system_date(text: &str, zone: Option<&str>) -> Option<u64> {
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

/// When each provider's usage limit resets (epoch ms).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cooldowns {
    until_ms: HashMap<ProviderId, u64>,
}

impl Cooldowns {
    /// Start (or restart) a cooldown after a quota error.
    pub fn start(&mut self, p: ProviderId, resets_at: Option<&str>, now_ms: u64) {
        self.start_with(p, resets_at, now_ms, &system_date);
    }

    /// [`Cooldowns::start`] with an injected wall-clock resolver.
    pub fn start_with(&mut self, p: ProviderId, resets_at: Option<&str>, now_ms: u64, resolve: Resolver) {
        self.until_ms.insert(p, now_ms.saturating_add(cooldown_ms_with(resets_at, now_ms, resolve)));
    }

    /// Milliseconds left if the provider is cooling down, else `None`.
    pub fn active(&self, p: ProviderId, now_ms: u64) -> Option<u64> {
        self.until_ms.get(&p).and_then(|u| u.checked_sub(now_ms)).filter(|left| *left > 0)
    }

    /// Forget a cooldown in this copy only. It does NOT lift a saved cooldown: once a
    /// cooldown has been saved it cannot be forgiven, it can only expire. That is on
    /// purpose. [`Cooldowns::save`] merges the file in (later expiry per provider
    /// wins) and [`crate::providers::Runtime`] unions the file into what it reports,
    /// because other processes share the file and one process must not erase a limit
    /// another just learned. So after `clear` the entry comes back from disk on the
    /// next save or read. Nothing in production calls this; it exists for tests and
    /// for dropping a throwaway copy.
    pub fn clear(&mut self, p: ProviderId) {
        self.until_ms.remove(&p);
    }

    /// Per provider, the later of the two expiries (expired entries kept).
    pub fn union(&self, other: &Self) -> Self {
        let mut until_ms = self.until_ms.clone();
        for (p, u) in &other.until_ms {
            let e = until_ms.entry(*p).or_insert(*u);
            *e = (*e).max(*u);
        }
        Self { until_ms }
    }

    /// The later expiry per provider across both sets, minus entries already over.
    pub fn merged(&self, other: &Self, now_ms: u64) -> Self {
        let mut m = self.union(other);
        m.until_ms.retain(|_, u| *u > now_ms);
        m
    }

    /// `{"claude": 1760000000000, ...}`: provider name to reset time.
    pub fn to_json(&self) -> Value {
        Value::Object(self.until_ms.iter().map(|(p, u)| (p.as_str().to_string(), Value::from(*u))).collect::<Map<_, _>>())
    }

    /// Tolerant reader: anything that is not a provider name with a number is skipped,
    /// so a damaged file costs at most the broken entries.
    pub fn from_json(v: &Value) -> Self {
        let until_ms = v
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.parse::<ProviderId>().ok()?, v.as_u64()?)))
            .collect();
        Self { until_ms }
    }

    /// Read `path`; a missing or garbled file is an empty set.
    pub fn load(path: &Path) -> Self {
        let value = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok());
        value.map_or_else(Self::default, |v| Self::from_json(&v))
    }

    /// Merge into the file and return what was written. Read-modify-write: other
    /// processes (`jev-code`, every `delegate_mcp`) share the file, so what is on disk
    /// is merged in (later expiry per provider wins) and entries already over at
    /// `now_ms` are dropped. Written atomically (temp file in the same directory, then
    /// rename) so a crash mid-write never leaves a half-written file. Writers that do
    /// not overlap in time can never lose each other's updates; a simultaneous pair
    /// can, within the few microseconds between the read and the rename.
    pub fn save(&self, path: &Path, now_ms: u64) -> std::io::Result<Self> {
        let merged = self.merged(&Self::load(path), now_ms);
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(".{}.tmp", std::process::id()));
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, merged.to_json().to_string())?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        Ok(merged)
    }
}

/// Cheap fingerprint of the cooldown file: modification time and length.
type FileStamp = (SystemTime, u64);

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The last copy of the cooldown file this process read, reloaded only when the file
/// changed (another process saved), so polling it costs one `stat`.
#[derive(Debug, Default)]
pub struct DiskWatch {
    stamp: Option<FileStamp>,
    disk: Cooldowns,
    reloads: usize,
}

impl DiskWatch {
    /// The file's current contents, re-read only if its stamp changed since the last
    /// call. A missing or garbled file is an empty set.
    pub fn refresh(&mut self, path: &Path) -> &Cooldowns {
        let stamp = file_stamp(path);
        if stamp != self.stamp {
            self.disk = Cooldowns::load(path);
            self.stamp = stamp;
            self.reloads += 1;
        }
        &self.disk
    }

    /// How many times the file was actually read.
    pub fn reloads(&self) -> usize {
        self.reloads
    }
}

/// `<state_dir>/cooldowns.json`, or `None` if there is no state directory.
pub fn default_path() -> Option<PathBuf> {
    Some(crate::log::state_dir()?.join("cooldowns.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use ProviderId::{Claude, Codex};

    const NOW: u64 = 1_760_000_000_000;

    /// A resolver that must never be reached.
    fn never(_: &str, _: Option<&str>) -> Option<u64> {
        panic!("the resolver must not be called");
    }

    fn unresolvable(_: &str, _: Option<&str>) -> Option<u64> {
        None
    }

    /// Only these zones "exist" in tests, so no test depends on the machine's zone database.
    fn fake_zones(zone: &str) -> bool {
        matches!(zone, "Asia/Jakarta" | "America/Los_Angeles")
    }

    fn cooldown_in(text: Option<&str>, now_ms: u64, resolve: Resolver) -> u64 {
        cooldown_ms_in(text, now_ms, resolve, &fake_zones)
    }

    #[test]
    fn default_is_one_hour() {
        // "tomorrow 9am" is a real date expression, so the resolver here knows nothing.
        for text in [None, Some(""), Some("tomorrow 9am"), Some("12abc")] {
            assert_eq!(cooldown_ms_with(text, NOW, &unresolvable), DEFAULT_MS, "{text:?}");
        }
    }

    #[test]
    fn parses_epoch_seconds_and_milliseconds() {
        let secs = ((NOW + 2 * HOUR_MS) / 1000).to_string();
        let ms = (NOW + 2 * HOUR_MS).to_string();
        assert_eq!(cooldown_ms_with(Some(&secs), NOW, &never), 2 * HOUR_MS);
        assert_eq!(cooldown_ms_with(Some(&format!(" {ms} ")), NOW, &never), 2 * HOUR_MS);
    }

    #[test]
    fn clamps_to_one_minute_and_eight_days() {
        let past = (NOW / 1000 - 5000).to_string();
        assert_eq!(cooldown_ms_with(Some(&past), NOW, &never), MIN_MS);
        let far = (NOW / 1000 + 90 * 86_400).to_string();
        assert_eq!(cooldown_ms_with(Some(&far), NOW, &never), MAX_MS);
        assert_eq!(cooldown_ms_with(Some("99999999999999999999999"), NOW, &never), DEFAULT_MS); // overflows u64
    }

    #[test]
    fn parses_durations_in_every_form_gemini_and_others_use() {
        let cases = [
            ("3h20m5s", 3 * HOUR_MS + 20 * MINUTE_MS + 5 * SECOND_MS),
            ("20m", 20 * MINUTE_MS),
            ("3h 20m", 3 * HOUR_MS + 20 * MINUTE_MS),
            ("45s", MINUTE_MS), // under the floor
            ("90 minutes", 90 * MINUTE_MS),
            ("2 hours", 2 * HOUR_MS),
            ("1 hour", HOUR_MS),
            ("in 5 hrs", 5 * HOUR_MS),
            ("1d", DAY_MS),
            ("2H30M", 2 * HOUR_MS + 30 * MINUTE_MS),
            ("30 days", MAX_MS),
        ];
        for (text, want) in cases {
            assert_eq!(cooldown_ms_with(Some(text), NOW, &never), want, "{text}");
        }
    }

    #[test]
    fn near_misses_are_not_durations() {
        for text in ["3x", "h", "3h20", "3h and 20m", "m5", "-5m", "5mm"] {
            assert_eq!(parse_duration_ms(text), None, "{text}");
        }
        assert_eq!(parse_duration_ms("99999999999999999999999d"), None); // overflows u64
        assert_eq!(parse_duration_ms(&format!("{}d", u64::MAX)), Some(u64::MAX)); // saturates, no panic
    }

    /// A resolver that returns a fixed instant and records what it was asked.
    fn fixed(at_secs: u64, seen: &RefCell<Vec<(String, Option<String>)>>) -> impl Fn(&str, Option<&str>) -> Option<u64> + '_ {
        move |text, zone| {
            seen.borrow_mut().push((text.to_string(), zone.map(str::to_string)));
            Some(at_secs)
        }
    }

    #[test]
    fn time_of_day_with_a_zone_goes_to_the_resolver() {
        let seen = RefCell::new(vec![]);
        let two_hours = fixed(NOW / 1000 + 7200, &seen);
        assert_eq!(cooldown_in(Some("5pm (Asia/Jakarta)"), NOW, &two_hours), 2 * HOUR_MS);
        assert_eq!(seen.borrow()[0], ("5pm".to_string(), Some("Asia/Jakarta".to_string())));
        assert_eq!(cooldown_ms_with(Some("5pm"), NOW, &two_hours), 2 * HOUR_MS);
        assert_eq!(seen.borrow()[1], ("5pm".to_string(), None));
    }

    #[test]
    fn a_bare_time_of_day_already_past_means_tomorrow() {
        let seen = RefCell::new(vec![]);
        let an_hour_ago = fixed(NOW / 1000 - 3600, &seen);
        for text in ["5pm (Asia/Jakarta)", "5:30 pm", "17:00 (UTC)"] {
            assert_eq!(cooldown_in(Some(text), NOW, &an_hour_ago), 23 * HOUR_MS, "{text}");
        }
        // Exactly now is not in the future either.
        assert_eq!(cooldown_ms_with(Some("5pm"), NOW, &fixed(NOW / 1000, &seen)), DAY_MS);
    }

    #[test]
    fn a_date_with_a_time_is_not_pushed_a_day() {
        let seen = RefCell::new(vec![]);
        let in_three_days = fixed(NOW / 1000 + 3 * 86_400, &seen);
        assert_eq!(cooldown_in(Some("Nov 20, 5pm (Asia/Jakarta)"), NOW, &in_three_days), 3 * DAY_MS);
        assert_eq!(seen.borrow()[0], ("Nov 20 5pm".to_string(), Some("Asia/Jakarta".to_string())));
        // In the past it is not "tomorrow": it clamps like any stale timestamp.
        let past = fixed(NOW / 1000 - 3600, &seen);
        assert_eq!(cooldown_in(Some("Nov 20, 5pm (Asia/Jakarta)"), NOW, &past), MIN_MS);
        // GNU date rejects the comma and "at"; both are dropped.
        cooldown_ms_with(Some("Nov 20 at 5pm"), NOW, &in_three_days);
        assert_eq!(seen.borrow().last().unwrap().0, "Nov 20 5pm");
    }

    #[test]
    fn iso_timestamps_go_to_the_resolver_and_clamp() {
        let seen = RefCell::new(vec![]);
        let tomorrow = fixed(NOW / 1000 + 86_400, &seen);
        assert_eq!(cooldown_ms_with(Some("2026-11-20T17:00:00Z"), NOW, &tomorrow), DAY_MS);
        assert_eq!(seen.borrow()[0], ("2026-11-20T17:00:00Z".to_string(), None));
        cooldown_ms_with(Some("2026-11-20T17:00:00.250+07:00"), NOW, &tomorrow);
        assert_eq!(seen.borrow()[1].0, "2026-11-20T17:00:00.250+07:00");
        assert_eq!(cooldown_ms_with(Some("2030-01-01T00:00:00Z"), NOW, &fixed(NOW / 1000 + 400 * 86_400, &seen)), MAX_MS);
    }

    /// 2026-12-31 12:00:00 UTC.
    const NEW_YEARS_EVE: u64 = 1_798_718_400_000;

    /// A resolver that behaves like `date` without a year: text with no year is read in
    /// 2026, text ending in a year is read in that year. Records every call.
    fn like_date(seen: &RefCell<Vec<String>>) -> impl Fn(&str, Option<&str>) -> Option<u64> + '_ {
        const JAN_2_5PM_2026: u64 = 1_767_373_200;
        const YEAR_SECS: u64 = 365 * 86_400;
        move |text, _| {
            seen.borrow_mut().push(text.to_string());
            let years_on = text.rsplit(' ').next().and_then(|w| w.parse::<u64>().ok()).map_or(0, |y| y - 2026);
            Some(JAN_2_5PM_2026 + years_on * YEAR_SECS)
        }
    }

    #[test]
    fn a_date_without_a_year_that_has_passed_means_next_year() {
        // Said on Dec 31: `date` reads "Jan 2 5pm" as Jan 2 of THIS year, 364 days ago.
        // Before the fix that clamped to one minute and the provider was retried every
        // minute for two days.
        let seen = RefCell::new(vec![]);
        let resolve = like_date(&seen);
        let want = 2 * DAY_MS + 5 * HOUR_MS; // Jan 2 17:00 minus Dec 31 12:00
        assert_eq!(cooldown_in(Some("Jan 2 5pm (Asia/Jakarta)"), NEW_YEARS_EVE, &resolve), want);
        assert_eq!(*seen.borrow(), ["Jan 2 5pm", "Jan 2 5pm 2027"]);
        assert_eq!(cooldown_in(Some("January 2, 5pm (UTC)"), NEW_YEARS_EVE, &resolve), want);
        assert_eq!(seen.borrow().last().unwrap(), "January 2 5pm 2027");
    }

    #[test]
    fn next_year_is_taken_only_when_in_the_future_and_within_the_cap() {
        let seen = RefCell::new(vec![]);
        // Still in the past next year too (a resolver that ignores the year): stale, one minute.
        let stale = fixed(NEW_YEARS_EVE / 1000 - 5, &seen);
        assert_eq!(cooldown_in(Some("Jan 2 5pm (UTC)"), NEW_YEARS_EVE, &stale), MIN_MS);
        // Next year but months away: not a reset anyone meant, not taken.
        let far = |text: &str, _: Option<&str>| Some(if text.ends_with("2027") { NEW_YEARS_EVE / 1000 + 62 * 86_400 } else { 1_000 });
        assert_eq!(cooldown_in(Some("Mar 3 5pm (UTC)"), NEW_YEARS_EVE, &far), MIN_MS);
        // Just inside the cap it is taken.
        let edge = |text: &str, _: Option<&str>| Some(if text.ends_with("2027") { NEW_YEARS_EVE / 1000 + 8 * 86_400 } else { 1_000 });
        assert_eq!(cooldown_in(Some("Jan 8 5pm (UTC)"), NEW_YEARS_EVE, &edge), MAX_MS);
    }

    #[test]
    fn a_date_that_names_its_year_or_is_not_a_date_is_not_asked_twice() {
        let seen = RefCell::new(vec![]);
        let past = fixed(NEW_YEARS_EVE / 1000 - 3600, &seen);
        for text in ["Jan 2 2026 5pm (UTC)", "2026-01-02T17:00:00Z", "tomorrow 9am"] {
            assert_eq!(cooldown_in(Some(text), NEW_YEARS_EVE, &past), MIN_MS, "{text}");
        }
        assert_eq!(seen.borrow().len(), 3, "one resolve each");
        // A bare time of day keeps its plus-one-day rule, never a year.
        assert_eq!(cooldown_in(Some("5pm (UTC)"), NEW_YEARS_EVE, &past), 23 * HOUR_MS);
        assert_eq!(seen.borrow().len(), 4);
    }

    #[test]
    fn year_of_an_instant_and_date_without_year_detection() {
        assert_eq!(utc_year(0), 1970);
        assert_eq!(utc_year(NEW_YEARS_EVE), 2026);
        assert_eq!(utc_year(NEW_YEARS_EVE + 12 * HOUR_MS), 2027);
        assert_eq!(utc_year(NEW_YEARS_EVE - 365 * DAY_MS), 2025);
        assert_eq!(utc_year(951_782_400_000), 2000); // 2000-02-29
        for yes in ["Jan 2 5pm", "January 2 17:00", "Nov 20 5:30pm", "Sept. 3 9am", "may 4 1pm"] {
            assert!(names_date_without_year(yes), "{yes}");
        }
        for no in ["Jan 2 2027 5pm", "5pm", "17:00", "2026-11-20T17:00:00Z", "tomorrow 9am", "mayor 5pm", "Monday 5pm"] {
            assert!(!names_date_without_year(no), "{no}");
        }
    }

    #[test]
    fn only_real_zones_are_accepted_abbreviations_and_unknown_names_are_the_default() {
        let seen = RefCell::new(vec![]);
        let two_hours = fixed(NOW / 1000 + 7200, &seen);
        // `TZ=PST date ...` exits 0 reading UTC, 8 hours wrong, so PST is not a zone.
        assert_eq!(cooldown_in(Some("5pm (PST)"), NOW, &two_hours), DEFAULT_MS);
        assert_eq!(cooldown_in(Some("5pm (Not/AZone)"), NOW, &two_hours), DEFAULT_MS);
        assert_eq!(cooldown_in(Some("5pm (EST)"), NOW, &two_hours), DEFAULT_MS);
        assert!(seen.borrow().is_empty(), "an unknown zone never reaches the resolver");
        assert_eq!(cooldown_in(Some("5pm (Asia/Jakarta)"), NOW, &two_hours), 2 * HOUR_MS);
        // Exactly UTC and GMT need no zone file.
        for zone in ["UTC", "GMT"] {
            assert_eq!(cooldown_ms_in(Some(&format!("5pm ({zone})")), NOW, &two_hours, &|_| false), 2 * HOUR_MS, "{zone}");
        }
        for zone in ["utc", "Utc", "UTC+3", "Z"] {
            assert_eq!(cooldown_ms_in(Some(&format!("5pm ({zone})")), NOW, &two_hours, &|_| false), DEFAULT_MS, "{zone}");
        }
        // No zone at all is local time and still goes to the resolver.
        assert_eq!(cooldown_in(Some("5pm"), NOW, &two_hours), 2 * HOUR_MS);
    }

    #[cfg(unix)]
    #[test]
    fn a_zone_must_be_a_tzif_file_inside_the_zone_directory() {
        let dir = temp_dir("zones");
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
        assert_eq!(system_date("5pm", Some("PST")), None);
        assert_eq!(system_date("5pm", Some("Not/AZone")), None);
    }

    #[test]
    fn what_the_resolver_cannot_read_is_the_default() {
        assert_eq!(cooldown_in(Some("5pm (Asia/Jakarta)"), NOW, &unresolvable), DEFAULT_MS);
    }

    #[test]
    fn unsafe_text_never_reaches_the_resolver_or_panics() {
        let long = "a".repeat(500);
        let bad = [
            "$(touch /tmp/pwned)",
            "`id`",
            "5pm; rm -rf /",
            "5pm && ls",
            "5pm | cat",
            "@1760000000",
            "\"5pm\"",
            "5pm\n6pm",
            "5pm (Asia/Jakarta; ls)",
            "5pm (../../etc/passwd)",
            "5pm (/etc/passwd)",
            "5pm (-x)",
            "5pm ()",
            "(Asia/Jakarta)",
            "5pm (Asia/Jakarta",
            "5pm )",
            "5pm (a) (b",
            "17 h\u{e9}",
            "\u{0}",
            &long,
        ];
        for text in bad {
            assert_eq!(cooldown_ms_with(Some(text), NOW, &never), DEFAULT_MS, "{text:?}");
        }
        // The real resolver refuses them too, without spawning anything.
        for text in bad {
            assert_eq!(system_date(text, None), None, "{text:?}");
        }
        assert_eq!(system_date("5pm", Some("../etc")), None);
        assert_eq!(system_date("5pm", Some("/etc/passwd")), None);
        assert_eq!(system_date("5pm", Some("")), None);
    }

    #[test]
    fn bare_time_of_day_detection() {
        for yes in ["5pm", "5 PM", "11:30am", "17:00", "5:30:15", "5pm"] {
            assert!(is_bare_time_of_day(yes), "{yes}");
        }
        for no in ["Nov 20, 5pm", "2026-11-20T17:00:00Z", "tomorrow", "5", "123pm", "", "pm", "5:300"] {
            assert!(!is_bare_time_of_day(no), "{no}");
        }
    }

    /// Runs the real GNU `date` once, on input that needs no time zone data; skipped
    /// where `date` is missing or not GNU (no `-d`).
    #[test]
    fn the_real_resolver_runs_date_when_it_exists() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let Some(at) = system_date("1 hour", None) else {
            eprintln!("skipped: no GNU date on this machine");
            return;
        };
        assert!((now + 3590..=now + 3700).contains(&at), "{at} vs {now}");
        // The forms Claude Code prints, after normalizing, are ones GNU date accepts.
        for text in ["5pm (Asia/Jakarta)", "Nov 20, 5pm (Asia/Jakarta)", "Nov 20 at 5:30pm (UTC)"] {
            let (body, zone) = split_zone(text).unwrap();
            if zone.is_some_and(|z| !zone_is_known(z, &system_zone_exists)) {
                eprintln!("skipped {text}: no zone database on this machine");
                continue;
            }
            assert!(system_date(&normalize_for_date(body), zone).is_some(), "{text}");
        }
    }

    #[test]
    fn active_counts_down_then_expires() {
        let mut c = Cooldowns::default();
        assert_eq!(c.active(Claude, NOW), None);
        c.start(Claude, None, NOW);
        assert_eq!(c.active(Claude, NOW), Some(DEFAULT_MS));
        assert_eq!(c.active(Claude, NOW + DEFAULT_MS - 1), Some(1));
        assert_eq!(c.active(Claude, NOW + DEFAULT_MS), None);
        assert_eq!(c.active(Codex, NOW), None);
    }

    #[test]
    fn start_with_uses_the_resolver() {
        let mut c = Cooldowns::default();
        c.start_with(Codex, Some("20m"), NOW, &never);
        assert_eq!(c.active(Codex, NOW), Some(20 * MINUTE_MS));
    }

    #[test]
    fn clear_removes() {
        let mut c = Cooldowns::default();
        c.start(Claude, None, NOW);
        c.clear(Claude);
        assert_eq!(c.active(Claude, NOW), None);
    }

    #[test]
    fn json_round_trip_and_garbage() {
        let mut c = Cooldowns::default();
        c.start(Claude, None, NOW);
        c.start(Codex, Some(&(NOW / 1000 + 7200).to_string()), NOW);
        assert_eq!(Cooldowns::from_json(&c.to_json()), c);
        for garbage in [serde_json::json!(null), serde_json::json!([1, 2]), serde_json::json!("x")] {
            assert_eq!(Cooldowns::from_json(&garbage), Cooldowns::default());
        }
        let partly = serde_json::json!({"claude": 5, "nobody": 6, "codex": "soon", "gemini": -1});
        let c = Cooldowns::from_json(&partly);
        assert_eq!(c.until_ms.len(), 1);
        assert_eq!(c.until_ms[&Claude], 5);
    }

    fn temp_dir(name: &str) -> PathBuf {
        crate::testkit::scratch_dir::scratch_dir("cooldown", name)
    }

    fn with(entries: &[(ProviderId, u64)]) -> Cooldowns {
        Cooldowns { until_ms: entries.iter().copied().collect() }
    }

    #[test]
    fn save_then_load_survives_a_restart_and_leaves_no_temp_file() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("cooldowns.json");
        let mut c = Cooldowns::default();
        c.start(Claude, None, NOW);
        c.save(&path, NOW).unwrap();
        assert_eq!(Cooldowns::load(&path), c);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_merges_with_the_file_keeping_the_later_expiry() {
        let dir = temp_dir("merge");
        let path = dir.join("cooldowns.json");
        with(&[(Claude, NOW + 5000), (Codex, NOW + 9000)]).save(&path, NOW).unwrap();
        // Another process knows a later Claude limit but nothing about Codex.
        let written = with(&[(Claude, NOW + 8000)]).save(&path, NOW).unwrap();
        assert_eq!(written, with(&[(Claude, NOW + 8000), (Codex, NOW + 9000)]));
        assert_eq!(Cooldowns::load(&path), written);
        // An earlier expiry never shortens what is on disk.
        with(&[(Claude, NOW + 100)]).save(&path, NOW).unwrap();
        assert_eq!(Cooldowns::load(&path).until_ms[&Claude], NOW + 8000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_drops_expired_entries_and_survives_a_garbage_file() {
        let dir = temp_dir("expired");
        let path = dir.join("cooldowns.json");
        std::fs::write(&path, "{not json").unwrap();
        let written = with(&[(Claude, NOW - 1), (Codex, NOW + 1)]).save(&path, NOW).unwrap();
        assert_eq!(written, with(&[(Codex, NOW + 1)]));
        assert_eq!(Cooldowns::load(&path), written);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_tolerates_missing_and_corrupt_files() {
        let dir = temp_dir("corrupt");
        assert_eq!(Cooldowns::load(&dir.join("absent.json")), Cooldowns::default());
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{not json").unwrap();
        assert_eq!(Cooldowns::load(&bad), Cooldowns::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disk_watch_reads_only_when_the_file_changed() {
        let dir = temp_dir("watch");
        let path = dir.join("cooldowns.json");
        let mut w = DiskWatch::default();
        // Missing file: empty, and the initial state already matches it, so no read.
        assert_eq!(*w.refresh(&path), Cooldowns::default());
        assert_eq!(w.reloads(), 0);

        with(&[(Claude, NOW + 1)]).save(&path, NOW).unwrap();
        assert_eq!(*w.refresh(&path), with(&[(Claude, NOW + 1)]));
        assert_eq!(w.reloads(), 1);
        for _ in 0..3 {
            w.refresh(&path);
        }
        assert_eq!(w.reloads(), 1, "unchanged file: no reload");

        // A change in size is seen even if the clock did not tick.
        with(&[(Claude, NOW + 1), (Codex, NOW + 2)]).save(&path, NOW).unwrap();
        assert_eq!(w.refresh(&path).until_ms.len(), 2);
        assert_eq!(w.reloads(), 2);

        // Garbage and deletion are tolerated and read as empty.
        std::fs::write(&path, "garbage that is longer than before").unwrap();
        assert_eq!(*w.refresh(&path), Cooldowns::default());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(*w.refresh(&path), Cooldowns::default());
        assert_eq!(w.reloads(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn golden_cooldowns() {
        let dir = std::env::temp_dir().join(format!("jev-golden-cooldowns-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let saved = |name: &str, before: Option<&str>, cd: &Cooldowns| {
            let path = dir.join(name);
            if let Some(text) = before {
                std::fs::write(&path, text).unwrap();
            }
            cd.save(&path, NOW).unwrap();
            std::fs::read_to_string(&path).unwrap()
        };
        let mut typical = Cooldowns::default();
        typical.start_with(Claude, Some("1760003600"), NOW, &never);
        typical.start_with(Codex, None, NOW, &unresolvable);
        // The file already holds an expired entry, a later one for the same provider, a number
        // that is not a time and a provider nobody knows: the save merges and drops what is over.
        let mut odd = Cooldowns::default();
        odd.start_with(Claude, Some("a quarter of an hour"), NOW, &unresolvable);
        let before = r#"{"claude": 1760000000001, "codex": 1759999999999, "gemini": 1760090000000, "grok": "soon", "nobody": 5}"#;
        let actual = format!(
            "## typical: two providers cooling down\n{}\n## unusual: merged into an existing file with expired, later and unreadable entries\n{}\n## unusual: nothing cooling down\n{}\n",
            saved("a.json", None, &typical),
            saved("b.json", Some(before), &odd),
            saved("c.json", None, &Cooldowns::default()),
        );
        let _ = std::fs::remove_dir_all(&dir);
        let golden = golden!("providers/cooldowns.txt");
        assert_eq!(golden.split_once('\n').unwrap().1, actual);
    }
}
