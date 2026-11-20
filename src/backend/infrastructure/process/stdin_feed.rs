//! The one task that writes to a CLI's stdin.

use tokio::io::AsyncWriteExt;
use tokio::process::ChildStdin;
use tokio::sync::mpsc;

/// Everything written to the CLI's stdin goes through one task, so a big prompt cannot
/// deadlock against a CLI that starts printing before it reads all of stdin, and so the
/// runner can keep writing (permission answers) while it reads stdout. Stdin closes when
/// the feed is closed or dropped; some CLIs wait forever for end of input.
pub(crate) struct StdinFeed(Option<mpsc::UnboundedSender<String>>);

impl StdinFeed {
    /// Send `payload`, then close stdin at once unless `keep_open`.
    pub(super) fn start(stdin: Option<ChildStdin>, payload: Option<String>, keep_open: bool) -> Self {
        let Some(mut stdin) = stdin else { return Self(None) };
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                // A CLI that exits early closes the pipe; its exit is reported by the runner.
                if stdin.write_all(text.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let mut feed = Self(Some(tx));
        if let Some(payload) = payload {
            feed.send(payload);
        }
        if !keep_open {
            feed.close();
        }
        feed
    }

    pub(super) fn send(&self, text: String) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(text);
        }
    }

    /// Close stdin once what was sent has been written.
    pub(super) fn close(&mut self) {
        self.0 = None;
    }
}
