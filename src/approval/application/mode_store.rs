//! Keeping the approval mode between sessions.

use std::path::Path;

use crate::approval::domain::mode::{self, Mode};
use crate::shared::application::host::Host;

/// Keep `mode` for the next session. Whole-file replace, so a crash never leaves half a word.
pub fn save(host: &dyn Host, state_dir: &Path, mode: Mode) -> std::io::Result<()> {
    host.write_private(&mode::saved_path(state_dir), mode.name())
}
