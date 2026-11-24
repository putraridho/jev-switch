//! Bounded-memory reading of an append-only log that rotates by rename (DESIGN 6.1). The
//! types and constants are frozen; T03 writes the bodies.

use jev_usage_core::usage::Unavailable;
use std::path::{Path, PathBuf};

/// The size of one read.
pub const BLOCK: usize = 256 * 1024;
/// The longest line that is delivered; a longer one is skipped to its newline.
pub const MAX_LINE: usize = 1024 * 1024;

/// Follows one log file and its previous generation `<path>.1`. Opens both read-only and
/// never creates, truncates, locks or writes either.
#[derive(Debug)]
pub struct TailReader {
    _path: PathBuf,
}

/// What one `read_new` saw.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum TailEvent {
    /// Nothing new.
    Unchanged,
    /// New lines were delivered.
    Read {
        /// How many.
        lines: u64,
    },
    /// The file was rotated; the old file was finished and the new one read.
    Rotated {
        /// How many lines were delivered.
        lines: u64,
    },
    /// The file does not exist.
    Missing,
}

impl TailReader {
    /// A reader for `path`; nothing is opened yet.
    pub fn new(path: PathBuf) -> Self {
        Self { _path: path }
    }

    /// Cold start (DESIGN 6.1). Scans backwards from EOF in blocks (at most `max_bytes`),
    /// calling `accept` for every complete line of each block read, and stops after the
    /// first block in which `accept` returned true at least once. Falls back to `<path>.1`.
    /// Leaves the reader positioned at EOF of `<path>`. Returns whether any line was accepted.
    pub fn seek_newest(
        &mut self,
        max_bytes: u64,
        accept: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<bool, Unavailable> {
        let _ = (max_bytes, accept);
        Err(Unavailable::Internal)
    }

    /// Feeds every complete new line since the last call, following a rotation.
    pub fn read_new(&mut self, on_line: &mut dyn FnMut(&[u8])) -> Result<TailEvent, Unavailable> {
        let _ = on_line;
        Err(Unavailable::Internal)
    }
}

/// Whole-file forward scan for the backfill; same line rules, constant memory. Returns the
/// number of lines delivered.
pub fn for_each_line(path: &Path, on_line: &mut dyn FnMut(&[u8])) -> Result<u64, Unavailable> {
    let _ = (path, on_line);
    Err(Unavailable::Internal)
}
