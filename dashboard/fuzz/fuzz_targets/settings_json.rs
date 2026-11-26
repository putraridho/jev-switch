#![no_main]

use libfuzzer_sys::fuzz_target;

use jev_usage_core::settings::{Settings, SettingsPatch};

fuzz_target!(|data: &[u8]| {
    if let Ok(settings) = serde_json::from_slice::<Settings>(data) {
        let _ = settings.validated();
    }
    if let Ok(patch) = serde_json::from_slice::<SettingsPatch>(data) {
        let _ = Settings::defaults().apply(patch);
    }
});
