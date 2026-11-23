//! A short local pause after a one-shot auxiliary model returns HTTP 429.

use std::time::{Duration, Instant};

use jev_catalog::domain::model_table::{MODEL_CHEAP, MODEL_MID};

const MODELS: [&str; 2] = [MODEL_CHEAP, MODEL_MID];
pub const AUXILIARY_COOLDOWN: Duration = Duration::from_secs(60);
pub const MAX_AUXILIARY_COOLDOWN: Duration = Duration::from_secs(300);

#[derive(Default)]
pub struct AuxiliaryCooldown {
    until: [Option<Instant>; 2],
}

impl AuxiliaryCooldown {
    pub fn select(&self, now: Instant) -> Result<&'static str, Duration> {
        if let Some(model) = MODELS.into_iter().find(|model| self.available(model, now)) {
            return Ok(model);
        }
        Err(self.until.iter().flatten().map(|until| until.duration_since(now)).min().unwrap())
    }

    pub fn available(&self, model: &str, now: Instant) -> bool {
        MODELS.iter().position(|candidate| *candidate == model)
            .and_then(|index| self.until[index])
            .is_none_or(|until| now >= until)
    }

    pub fn reject_rate_limited(&mut self, model: &str, retry_after: Option<Duration>, now: Instant) {
        if let Some(index) = MODELS.iter().position(|candidate| *candidate == model) {
            let pause = retry_after.unwrap_or(AUXILIARY_COOLDOWN).clamp(Duration::from_secs(1), MAX_AUXILIARY_COOLDOWN);
            self.until[index] = Some(now + pause);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limits_pause_only_rejected_models_until_the_local_deadline() {
        let now = Instant::now();
        let mut cooldown = AuxiliaryCooldown::default();
        assert_eq!(cooldown.select(now), Ok(MODEL_CHEAP));
        cooldown.reject_rate_limited(MODEL_CHEAP, None, now);
        assert_eq!(cooldown.select(now), Ok(MODEL_MID));
        cooldown.reject_rate_limited(MODEL_MID, None, now + Duration::from_secs(1));
        assert_eq!(cooldown.select(now + Duration::from_secs(2)), Err(Duration::from_secs(58)));
        assert_eq!(cooldown.select(now + AUXILIARY_COOLDOWN), Ok(MODEL_CHEAP));
    }

    #[test]
    fn other_models_do_not_change_auxiliary_cooldowns() {
        let now = Instant::now();
        let mut cooldown = AuxiliaryCooldown::default();
        cooldown.reject_rate_limited("other-model", None, now);
        assert_eq!(cooldown.select(now), Ok(MODEL_CHEAP));
        cooldown.reject_rate_limited(MODEL_CHEAP, None, now);
        cooldown.reject_rate_limited("other-model", None, now);
        assert_eq!(cooldown.select(now), Ok(MODEL_MID));
    }

    #[test]
    fn retry_after_is_used_with_a_bounded_local_pause() {
        let now = Instant::now();
        for (hint, expected) in [(0, 1), (17, 17), (300, 300), (u64::MAX, 300)] {
            let mut cooldown = AuxiliaryCooldown::default();
            for model in MODELS {
                cooldown.reject_rate_limited(model, Some(Duration::from_secs(hint)), now);
            }
            assert_eq!(cooldown.select(now), Err(Duration::from_secs(expected)));
            assert_eq!(cooldown.select(now + Duration::from_secs(expected)), Ok(MODEL_CHEAP));
        }
    }
}
