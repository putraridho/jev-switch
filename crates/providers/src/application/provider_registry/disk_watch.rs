use std::path::Path;
use std::time::SystemTime;

use crate::application::ports::ProviderHost;
use crate::domain::reset_time::cooldowns::Cooldowns;

#[derive(Debug, Default)]
pub struct DiskWatch {
    stamp: Option<(SystemTime, u64)>,
    disk: Cooldowns,
    reloads: usize,
}

impl DiskWatch {
    pub fn refresh(&mut self, path: &Path, host: &dyn ProviderHost) -> &Cooldowns {
        let stamp = host.cooldown_stamp(path);
        if stamp != self.stamp {
            self.disk = host.load_cooldowns(path);
            self.stamp = stamp;
            self.reloads += 1;
        }
        &self.disk
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn reloads(&self) -> usize {
        self.reloads
    }
}
