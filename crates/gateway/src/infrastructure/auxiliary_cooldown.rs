//! Translate numeric Retry-After headers to a duration for the local cooldown policy.

use std::time::Duration;

pub fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers.get("retry-after")?.to_str().ok()?.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_retry_after_is_parsed_without_guessing_other_formats() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert_eq!(retry_after(&headers), None);
        for (value, expected) in [("17", Some(17)), (" 30 ", Some(30)), ("0", Some(0)), ("-1", None), ("1.5", None), ("invalid", None), ("Wed, 21 Oct 2015 07:28:00 GMT", None)] {
            headers.insert("retry-after", value.parse().unwrap());
            assert_eq!(retry_after(&headers), expected.map(Duration::from_secs));
        }
    }
}
