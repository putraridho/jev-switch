//! Process hardening at start (DESIGN 8.8, 8.4): umask 077, `RLIMIT_CORE` 0,
//! `PR_SET_DUMPABLE` 0 and the panic hook (base name and line only). T15 writes the body.

/// Applies the hardening. Called first, before any thread or child exists.
pub fn apply() {}
