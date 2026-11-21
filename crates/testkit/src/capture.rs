//! A capture writer: an `io::Write` whose bytes a test reads back afterwards.

use std::io::Write;
use std::sync::{Arc, Mutex};

/// A writer whose bytes can be read back. Clones share the same buffer.
#[derive(Clone, Default)]
pub struct Buf(pub Arc<Mutex<Vec<u8>>>);

impl Buf {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
