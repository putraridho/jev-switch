//! Keeping what turns showed about each prompt cache between sessions: counts and gaps per
//! (provider, model, backend), never prompt text. Written through the host's private,
//! never-half-written file replacement (the same guarantee as the other state files).

use std::path::Path;

use jev_routing::domain::cache_facts::{ CacheBook, FILE };
use jev_shared::application::host::Host;

/// The facts kept from earlier sessions; none when there is no readable file. Facts older
/// than the limit are left out.
pub fn load(host: &dyn Host, state_dir: Option<&Path>, now_ms: u64) -> CacheBook {
    let text = state_dir.and_then(|d| host.read_text(&d.join(FILE)).ok()).unwrap_or_default();
    CacheBook::parse(&text, now_ms)
}

/// Merge `book` with what another session saved meanwhile (the record that changed last wins
/// per key), write the result and return it. Without a state folder nothing is written.
pub fn save(host: &dyn Host, book: &CacheBook, state_dir: Option<&Path>, now_ms: u64) -> CacheBook {
    let Some(dir) = state_dir else {
        return book.clone();
    };
    let merged = load(host, Some(dir), now_ms).merged(book).expired(now_ms);
    // A failed write only means the next session starts from what is on disk.
    let _ = host.write_private(&dir.join(FILE), &merged.file_text());
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_routing::domain::cache_facts::CacheKey;
    use jev_shared::infrastructure::system_host::SystemHost;
    use jev_shared::{ ProviderId, Usage };

    fn book(hit: u64, now: u64) -> CacheBook {
        let mut book = CacheBook::default();
        let usage = Usage {
            input_tokens: 1_000,
            cached_input_tokens: hit,
            output_tokens: 1,
            ..Usage::default()
        };
        book.observe(&CacheKey::new(ProviderId::Claude, "m", "Claude login"), &usage, None, now);
        book
    }

    #[test]
    fn facts_survive_a_session_and_hold_no_prompt_text() {
        let dir = jev_testkit::scratch_dir::scratch_dir("cache-store", "round");
        let now = SystemHost.now_ms();
        assert!(load(&SystemHost, Some(&dir), now).is_empty());
        let saved = save(&SystemHost, &book(900, now), Some(&dir), now);
        assert_eq!(load(&SystemHost, Some(&dir), now), saved);
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.starts_with("claude\tm\tClaude login\t1000\t900\t1\t"), "{text}");
        assert!(load(&SystemHost, None, now).is_empty());
        assert_eq!(
            save(&SystemHost, &book(1, now), None, now),
            book(1, now),
            "no folder: kept in memory only"
        );
    }

    #[test]
    fn saving_keeps_what_another_session_saved_and_forgets_the_expired() {
        let dir = jev_testkit::scratch_dir::scratch_dir("cache-store", "merge");
        let now = SystemHost.now_ms();
        save(&SystemHost, &book(900, now + 10), Some(&dir), now);
        let mine = save(&SystemHost, &book(0, now), Some(&dir), now);
        let key = CacheKey::new(ProviderId::Claude, "m", "Claude login");
        assert_eq!(
            mine.get(&key).unwrap().hit_percent(),
            Some(90),
            "the newer record is the other session's"
        );
        std::fs::write(dir.join(FILE), "claude\tm\tb\t10\t5\t1\t0\t-\t1\n").unwrap();
        assert!(load(&SystemHost, Some(&dir), now).is_empty(), "a record from 1970 is expired");
    }
}
