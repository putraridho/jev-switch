//! Stopping one turn without stopping the program.

use std::sync::Arc;

use tokio::sync::watch;

/// A switch that stops one turn without stopping the program (Ctrl-C in the UI). Cheap
/// to clone; every clone sees the same state. Once cancelled it stays cancelled, so make
/// a new one for each turn.
#[derive(Clone, Debug)]
pub struct Cancel {
    tx: Arc<watch::Sender<bool>>,
    rx: watch::Receiver<bool>,
}

impl Cancel {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self { tx: Arc::new(tx), rx }
    }

    /// Ask the turn to stop. Safe to call more than once, from any thread.
    pub fn cancel(&self) {
        self.tx.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }

    /// Resolves once [`Cancel::cancel`] has been called (at once if it already was).
    pub async fn cancelled(&self) {
        let mut rx = self.rx.clone();
        if rx.wait_for(|c| *c).await.is_err() {
            // Unreachable while `self` holds the sender; never report a cancel that did not happen.
            std::future::pending::<()>().await;
        }
    }
}

/// Resolves when `cancel` fires, or never when `honor` is false.
pub(super) async fn stop_signal(cancel: &Cancel, honor: bool) {
    if honor {
        cancel.cancelled().await;
    } else {
        std::future::pending::<()>().await;
    }
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}
