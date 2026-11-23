//! The usage ledger as a JSON file in the state folder, shared by every jev-code and
//! `delegate_mcp` process. A change is a locked read-modify-write: the process takes an
//! exclusive lock on `<file>.lock` (`flock`, released by the system if the process dies),
//! reads the file, changes it, and replaces it with a rename, so a reader never sees half a
//! write and two writers never lose each other's change.
//!
//! The file is the user's own record of spend; a file that cannot be parsed is moved aside to
//! `<file>.bad` and a new one started, rather than guessed at.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use jev_backend::{ProviderId, Usage};
use jev_shared::Measurement;

use crate::application::ports::UsageLedger;
use crate::domain::usage_ledger::{Book, Held, Session, Spent};

pub use crate::application::ports::USAGE_LEDGER_FILE as FILE;

#[derive(Serialize, Deserialize, Default)]
struct BookWire {
    #[serde(default)]
    current: BTreeMap<String, String>,
    #[serde(default)]
    sessions: BTreeMap<String, SessionWire>,
}

#[derive(Serialize, Deserialize, Default)]
struct SessionWire {
    #[serde(default)]
    project: String,
    #[serde(default)]
    started_ms: u64,
    #[serde(default)]
    held: Vec<HeldWire>,
    #[serde(default)]
    spent: Vec<SpentWire>,
}

#[derive(Serialize, Deserialize)]
struct HeldWire {
    id: String,
    pid: u32,
    #[serde(default)]
    start: Option<u64>,
    ts_ms: u64,
    provider: String,
    model: String,
    #[serde(default)]
    paid: bool,
    #[serde(default)]
    tokens: u64,
    #[serde(default)]
    usd: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct SpentWire {
    ts_ms: u64,
    writer: String,
    provider: String,
    model: String,
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    unreported: bool,
}

impl From<&Book> for BookWire {
    fn from(book: &Book) -> Self {
        let sessions = book
            .sessions
            .iter()
            .map(|(id, s)| {
                let held = s
                    .held
                    .iter()
                    .map(|h| HeldWire { id: h.id.clone(), pid: h.owner_pid, start: h.owner_start, ts_ms: h.ts_ms, provider: h.provider.as_str().into(), model: h.model.clone(), paid: h.paid, tokens: h.tokens, usd: h.usd })
                    .collect();
                let spent = s
                    .spent
                    .iter()
                    .map(|p| SpentWire {
                        ts_ms: p.ts_ms,
                        writer: p.writer.clone(),
                        provider: p.provider.as_str().into(),
                        model: p.model.clone(),
                        input_tokens: p.usage.input_tokens,
                        cached_input_tokens: p.usage.cached_input_tokens,
                        output_tokens: p.usage.output_tokens,
                        unreported: p.usage.measurement == Measurement::Unavailable,
                    })
                    .collect();
                (id.clone(), SessionWire { project: s.project.clone(), started_ms: s.started_ms, held, spent })
            })
            .collect();
        Self { current: book.current.clone(), sessions }
    }
}

impl From<BookWire> for Book {
    /// Entries of a provider this version does not know are dropped.
    fn from(wire: BookWire) -> Self {
        let sessions = wire
            .sessions
            .into_iter()
            .map(|(id, s)| {
                let held = s
                    .held
                    .into_iter()
                    .filter_map(|h| {
                        Some(Held { id: h.id, owner_pid: h.pid, owner_start: h.start, ts_ms: h.ts_ms, provider: h.provider.parse::<ProviderId>().ok()?, model: h.model, paid: h.paid, tokens: h.tokens, usd: h.usd })
                    })
                    .collect();
                let spent = s
                    .spent
                    .into_iter()
                    .filter_map(|p| {
                        Some(Spent {
                            ts_ms: p.ts_ms,
                            writer: p.writer,
                            provider: p.provider.parse::<ProviderId>().ok()?,
                            model: p.model,
                            usage: Usage {
                                measurement: if p.unreported { Measurement::Unavailable } else { Measurement::Reported },
                                input_tokens: p.input_tokens,
                                cached_input_tokens: p.cached_input_tokens,
                                output_tokens: p.output_tokens,
                                ..Usage::default() },
                        })
                    })
                    .collect();
                (id, Session { project: s.project, started_ms: s.started_ms, held, spent })
            })
            .collect();
        Self { current: wire.current, sessions }
    }
}

/// The ledger kept in a file; a handle holds no state, so any number of them may name one file.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileUsageLedger;

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// An exclusive lock on the ledger, released when dropped (or when the process dies).
struct Locked {
    _file: File,
}

impl Locked {
    fn take(path: &Path) -> std::io::Result<Self> {
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).write(true).read(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(lock_path(path))?;
        lock_exclusive(&file)?;
        Ok(Self { _file: file })
    }
}

#[cfg(unix)]
fn lock_exclusive(file: &File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    loop {
        // SAFETY: `flock` on a descriptor this function owns for the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(not(unix))]
fn lock_exclusive(_file: &File) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no file lock on this platform: the usage ledger is not shared"))
}

fn load(path: &Path) -> Book {
    let Ok(text) = std::fs::read_to_string(path) else { return Book::default() };
    match serde_json::from_str::<BookWire>(&text) {
        Ok(wire) => wire.into(),
        Err(_) => {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            let _ = std::fs::rename(path, PathBuf::from(bad));
            Book::default()
        }
    }
}

fn store(path: &Path, book: &Book) -> std::io::Result<()> {
    let text = serde_json::to_string(&BookWire::from(book)).map_err(std::io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&tmp)?, text.as_bytes())?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

impl UsageLedger for FileUsageLedger {
    fn transact(&self, path: &Path, change: &mut dyn FnMut(&mut Book)) -> bool {
        let Ok(_lock) = Locked::take(path) else { return false };
        let mut book = load(path);
        change(&mut book);
        store(path, &book).is_ok()
    }

    fn read(&self, path: &Path) -> Option<Book> {
        if !path.exists() {
            return Some(Book::default());
        }
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<BookWire>(&text).ok().map(Book::from)
    }
}

#[cfg(test)]
mod tests;
