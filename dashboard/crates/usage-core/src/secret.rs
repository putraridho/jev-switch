//! Secrets in memory (DESIGN 8.5).

use zeroize::Zeroize;

/// Holds a secret. Deliberately implements NONE of: `Debug`, `Display`, `Clone`,
/// `Serialize`, `PartialEq`, `Hash`. A struct that contains one cannot derive them either,
/// so a secret cannot be printed, logged, compared, cloned or serialized by accident: it
/// does not compile.
pub struct Secret<T: Zeroize>(T);

impl<T: Zeroize> Secret<T> {
    /// Takes ownership of the secret value.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// The single way out. Call sites are counted by a test: exactly one, in `io::http`,
    /// where the Authorization header is built.
    pub fn expose_secret(&self) -> &T {
        &self.0
    }
}

impl<T: Zeroize> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;
    use serde::Serialize;
    use static_assertions::{assert_impl_all, assert_not_impl_any};
    use std::fmt::{Debug, Display};
    use std::hash::Hash;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    assert_not_impl_any!(Secret<String>: Debug, Display, Clone, Copy, Serialize, PartialEq, Eq, Hash, Default);
    assert_not_impl_any!(Secret<Vec<u8>>: Debug, Display, Clone, Serialize, PartialEq, Hash);
    assert_impl_all!(Secret<String>: Send, Sync);

    #[test]
    fn expose_returns_the_value() {
        let secret = Secret::new(String::from("JEVCANARY_KEY_test"));
        assert_eq!(secret.expose_secret(), "JEVCANARY_KEY_test");
    }

    /// Records how often it was zeroized, so the test can see `Drop` doing it.
    struct Recorder(Arc<AtomicUsize>);
    impl Zeroize for Recorder {
        fn zeroize(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn drop_zeroizes_exactly_once() {
        let count = Arc::new(AtomicUsize::new(0));
        let secret = Secret::new(Recorder(Arc::clone(&count)));
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let _ = secret.expose_secret();
        assert_eq!(count.load(Ordering::SeqCst), 0);
        drop(secret);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
