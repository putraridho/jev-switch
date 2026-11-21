//! Shared fixtures for the session library tests.

use super::search::Hit;
use super::*;
use jev_backend::ProviderId;
use crate::domain::saved_session::SavedSession;
use crate::domain::session::Sessions;
use jev_shared::infrastructure::private_file::write_atomic;
use crate::infrastructure::session_file::{self, FileSessions};
pub(super) use crate::infrastructure::library_fixtures::{library, put, temp, turn, NOW};
use jev_shared::infrastructure::system_host::SystemHost;

mod searching;
mod export_import;
mod disk_and_clean;

pub(super) fn titles(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|h| h.entry.title()).collect()
}
