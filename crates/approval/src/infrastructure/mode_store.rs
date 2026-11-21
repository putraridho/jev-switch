//! Reading the approval mode kept between sessions.

use std::path::Path;

use crate::domain::mode::{saved_path, Mode};

/// A missing, unreadable, or unrecognized value is no choice at all.
pub fn load_saved(state_dir: &Path) -> Option<Mode> {
    std::fs::read_to_string(saved_path(state_dir)).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mode_store::save;
    use jev_shared::infrastructure::system_host::SystemHost;

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
        let nested = dir.join("a/b");
        save(&SystemHost, &nested, Mode::Auto).unwrap();
        assert_eq!(load_saved(&nested), Some(Mode::Auto));
        assert_eq!(std::fs::read_dir(&nested).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
