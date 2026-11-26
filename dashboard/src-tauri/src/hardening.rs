//! Process hardening at start (DESIGN 8.8, 8.4): umask 077, `RLIMIT_CORE` 0,
//! `PR_SET_DUMPABLE` 0 and the panic hook (base name and line only). T15 writes the body.

/// Applies the hardening. Called first, before any thread or child exists.
pub fn apply() {
    use rustix::fs::Mode;
    use rustix::process::{
        set_dumpable_behavior, setrlimit, umask, DumpableBehavior, Resource, Rlimit,
    };

    // umask 077: files created by the app default to private.
    let _ = umask(Mode::from_bits_truncate(0o077));
    // No core dumps.
    let _ = setrlimit(
        Resource::Core,
        Rlimit {
            current: Some(0),
            maximum: Some(0),
        },
    );
    // The process is not dumpable by other users.
    let _ = set_dumpable_behavior(DumpableBehavior::NotDumpable);
    // The panic hook prints a base name and a line number, never a path or the payload.
    std::panic::set_hook(Box::new(|info| {
        use std::io::Write;
        let (file, line) = info
            .location()
            .map_or(("<unknown>", 0), |l| (l.file(), l.line()));
        let base = file.rsplit('/').next().unwrap_or(file);
        let msg = format!("panicked at {base}:{line}\n");
        let _ = std::io::stderr().write_all(msg.as_bytes());
    }));
}
