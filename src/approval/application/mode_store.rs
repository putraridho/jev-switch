//! Keeping the approval mode between sessions: the writing half of [`mode::load_saved`].

use std::path::Path;

use crate::approval::domain::mode::{self, Mode};
use crate::shared::application::host::Host;

/// Keep `mode` for the next session. Whole-file replace, so a crash never leaves half a word.
pub fn save(host: &dyn Host, state_dir: &Path, mode: Mode) -> std::io::Result<()> {
    host.write_private(&mode::saved_path(state_dir), mode.name())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::infrastructure::system_host::SystemHost;
    use crate::approval::domain::mode::{load_saved, saved_path};

    #[test]
    fn a_kept_mode_survives_and_a_bad_file_is_no_choice() {
        let dir = std::env::temp_dir().join(format!("jev-mode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load_saved(&dir), None);
        for m in Mode::ALL {
            save(&SystemHost, &dir, m).unwrap();
            assert_eq!(load_saved(&dir), Some(m));
        }
        std::fs::write(saved_path(&dir), "bypassPermissions").unwrap();
        assert_eq!(load_saved(&dir), None);
        std::fs::write(saved_path(&dir), "").unwrap();
        assert_eq!(load_saved(&dir), None);
        // A folder that does not exist yet is made, and nothing else is left in it.
        let nested = dir.join("a/b");
        save(&SystemHost, &nested, Mode::Auto).unwrap();
        assert_eq!(load_saved(&nested), Some(Mode::Auto));
        assert_eq!(std::fs::read_dir(&nested).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
