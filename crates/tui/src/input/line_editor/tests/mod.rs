use super::*;
use jev_approval::domain::mode::SharedMode;

mod approval;
mod completion;
mod helper;
mod history;
mod interrupt;
mod keys;

pub(super) fn temp(name: &str) -> PathBuf {
    jev_testkit::scratch_dir::scratch_dir("lineedit", name)
}
