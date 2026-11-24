//! Bounded-memory reading of an append-only log that rotates by rename (DESIGN 6.1). The
//! types and constants are frozen; T03 writes the bodies.

use jev_usage_core::usage::Unavailable;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// The size of one read.
pub const BLOCK: usize = 256 * 1024;
/// The longest line that is delivered; a longer one is skipped to its newline.
pub const MAX_LINE: usize = 1024 * 1024;

/// Follows one log file and its previous generation `<path>.1`. Opens both read-only and
/// never creates, truncates, locks or writes either.
#[derive(Debug)]
pub struct TailReader {
    path: PathBuf,
    file: Option<File>,
    identity: Option<(u64, u64)>,
    offset: u64,
    carry: Zeroizing<Vec<u8>>,
    skipping: bool,
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
        Self {
            path,
            file: None,
            identity: None,
            offset: 0,
            carry: Zeroizing::new(Vec::with_capacity(MAX_LINE)),
            skipping: false,
        }
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
        let mut accepted = false;
        let mut remaining = max_bytes;
        for path in [self.path.clone(), previous(&self.path)] {
            let mut file = match File::open(path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(Unavailable::Io),
            };
            let size = file.metadata().map_err(|_| Unavailable::Io)?.len();
            let mut end = size;
            let mut suffix = Zeroizing::new(Vec::new());
            while end > 0 && remaining > 0 {
                let amount = end
                    .min(remaining)
                    .min(u64::try_from(BLOCK).map_err(|_| Unavailable::Internal)?);
                let start = end.saturating_sub(amount);
                file.seek(SeekFrom::Start(start))
                    .map_err(|_| Unavailable::Io)?;
                let mut block = Zeroizing::new(vec![
                    0;
                    usize::try_from(amount)
                        .map_err(|_| Unavailable::Internal)?
                ]);
                file.read_exact(&mut block).map_err(|_| Unavailable::Io)?;
                remaining = remaining.saturating_sub(amount);
                // A partial line at the right edge belongs to the next block.
                let mut combined =
                    Zeroizing::new(Vec::with_capacity(block.len().saturating_add(suffix.len())));
                combined.extend_from_slice(&block);
                combined.extend_from_slice(&suffix);
                let first = if start == 0 {
                    0
                } else {
                    combined
                        .iter()
                        .position(|b| *b == b'\n')
                        .map_or(combined.len(), |i| i.saturating_add(1))
                };
                let mut found = false;
                if let Some(rest) = combined.get(first..) {
                    for line in rest.split_inclusive(|b| *b == b'\n') {
                        if let Some(line) = line.strip_suffix(b"\n") {
                            if line.len() <= MAX_LINE && accept(line) {
                                found = true;
                            }
                        }
                    }
                }
                suffix.clear();
                if start > 0 && first <= MAX_LINE {
                    if let Some(prefix) = combined.get(..first) {
                        suffix.extend_from_slice(prefix);
                    }
                }
                end = start;
                if found {
                    accepted = true;
                    break;
                }
            }
            if accepted {
                break;
            }
        }
        self.file = None;
        self.identity = None;
        self.offset = 0;
        self.carry.clear();
        self.skipping = false;
        match File::open(&self.path) {
            Ok(mut file) => {
                let metadata = file.metadata().map_err(|_| Unavailable::Io)?;
                self.offset = metadata.len();
                self.identity = Some((metadata.dev(), metadata.ino()));
                file.seek(SeekFrom::Start(self.offset))
                    .map_err(|_| Unavailable::Io)?;
                self.file = Some(file);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Unavailable::Io),
        }
        Ok(accepted)
    }

    /// Feeds every complete new line since the last call, following a rotation.
    pub fn read_new(&mut self, on_line: &mut dyn FnMut(&[u8])) -> Result<TailEvent, Unavailable> {
        let current = match File::open(&self.path) {
            Ok(file) => Some(file),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(Unavailable::Io),
        };
        let metadata = current
            .as_ref()
            .map(File::metadata)
            .transpose()
            .map_err(|_| Unavailable::Io)?;
        let identity = metadata.as_ref().map(|m| (m.dev(), m.ino()));
        let rotated = self.identity.is_some()
            && (identity != self.identity
                || metadata.as_ref().is_some_and(|m| m.len() < self.offset));
        let mut lines = 0_u64;
        if rotated {
            if identity != self.identity {
                if let Some(mut old) = self.file.take() {
                    lines = lines.saturating_add(read_lines(
                        &mut old,
                        &mut self.offset,
                        &mut self.carry,
                        &mut self.skipping,
                        on_line,
                    )?);
                }
            }
            self.carry.clear();
            self.skipping = false;
            self.offset = 0;
            self.file = current;
            self.identity = identity;
        } else if self.file.is_none() {
            self.file = current;
            self.identity = identity;
        }
        if let Some(file) = self.file.as_mut() {
            lines = lines.saturating_add(read_lines(
                file,
                &mut self.offset,
                &mut self.carry,
                &mut self.skipping,
                on_line,
            )?);
        }
        if rotated {
            Ok(TailEvent::Rotated { lines })
        } else if self.file.is_none() {
            Ok(TailEvent::Missing)
        } else if lines > 0 {
            Ok(TailEvent::Read { lines })
        } else {
            Ok(TailEvent::Unchanged)
        }
    }
}

/// Whole-file forward scan for the backfill; same line rules, constant memory. Returns the
/// number of lines delivered.
pub fn for_each_line(path: &Path, on_line: &mut dyn FnMut(&[u8])) -> Result<u64, Unavailable> {
    let mut file = File::open(path).map_err(|_| Unavailable::Io)?;
    let mut offset = 0;
    let mut carry = Zeroizing::new(Vec::with_capacity(MAX_LINE));
    let mut skipping = false;
    read_lines(&mut file, &mut offset, &mut carry, &mut skipping, on_line)
}

fn previous(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".1");
    PathBuf::from(name)
}

fn read_lines(
    file: &mut File,
    offset: &mut u64,
    carry: &mut Zeroizing<Vec<u8>>,
    skipping: &mut bool,
    on_line: &mut dyn FnMut(&[u8]),
) -> Result<u64, Unavailable> {
    file.seek(SeekFrom::Start(*offset))
        .map_err(|_| Unavailable::Io)?;
    let mut buffer = Zeroizing::new(vec![0_u8; BLOCK]);
    let mut lines = 0_u64;
    loop {
        let count = file.read(&mut buffer).map_err(|_| Unavailable::Io)?;
        if count == 0 {
            break;
        }
        *offset = offset.saturating_add(u64::try_from(count).map_err(|_| Unavailable::Internal)?);
        for byte in buffer.iter().take(count) {
            if *byte == b'\n' {
                if !*skipping {
                    on_line(carry);
                    lines = lines.saturating_add(1);
                }
                carry.clear();
                *skipping = false;
            } else if !*skipping {
                if carry.len() == MAX_LINE {
                    carry.clear();
                    *skipping = true;
                } else {
                    carry.push(*byte);
                }
            }
        }
    }
    Ok(lines)
}
