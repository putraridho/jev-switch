//! Shared fixtures for the session library tests.

use super::search::Hit;
use super::*;
use crate::backend::ProviderId;
use crate::session::domain::saved_session::SavedSession;
use crate::session::domain::session::Sessions;
use crate::shared::infrastructure::private_file::write_atomic;
use crate::session::infrastructure::session_file::{self, FileSessions};
pub(super) use crate::session::infrastructure::library_fixtures::{library, put, temp, turn, NOW};
use crate::shared::infrastructure::system_host::SystemHost;

mod searching;
mod export_import;
mod disk_and_clean;

pub(super) fn titles(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|h| h.entry.title()).collect()
}
