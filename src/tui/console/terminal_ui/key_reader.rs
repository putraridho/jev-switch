use super::*;


/// One key still to come. Dropping it stops the wait at once, without blocking; the terminal
/// is put back as it was by the reader itself (see [`terminal_keys`]).
pub type KeyWait = Pin<Box<dyn Future<Output = AnswerKey> + Send>>;

/// Starts waiting for one key at an approval question; `true` says "always" can be answered.
pub type KeyReader = Arc<dyn Fn(bool) -> KeyWait + Send + Sync>;

/// The real terminal's key reader: [`lineedit::read_answer`] on a thread of its own, which
/// polls stdin and a cancel flag. Dropping the [`KeyWait`] (the question was withdrawn) sets
/// the flag and returns at once, without blocking the async runtime: the thread notices
/// within one poll interval and puts the terminal back in the mode it was in before the
/// question itself (it never leaves raw mode behind). Until then it is "pending":
/// whoever touches the terminal mode next waits for it first ([`settle_key_readers`]), so
/// two readers never overlap and a restore cannot land after the next change.
pub fn terminal_keys() -> KeyReader {
    Arc::new(|always| Box::pin(TerminalKey::start(always)))
}

/// Readers whose question was withdrawn and that are still restoring the terminal.
pub(super) static PENDING_READERS: Mutex<Vec<std::thread::JoinHandle<()>>> = Mutex::new(Vec::new());

/// Wait for every withdrawn key reader to have put the terminal back (at most one poll
/// interval each). Blocking: called on a reader's own thread, or through
/// [`settle_key_readers_from_async`].
pub(super) fn settle_key_readers() {
    let pending = std::mem::take(&mut *PENDING_READERS.lock().unwrap_or_else(PoisonError::into_inner));
    for reader in pending {
        let _ = reader.join();
    }
}

/// [`settle_key_readers`] from code that may run on the async runtime: told to the runtime
/// as blocking where it can be (a multi-thread runtime), else run directly. Free when
/// nothing is pending, which is every time no question was withdrawn.
pub(super) fn settle_key_readers_from_async() {
    if PENDING_READERS.lock().unwrap_or_else(PoisonError::into_inner).is_empty() {
        return;
    }
    let multi_thread = tokio::runtime::Handle::try_current().is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread);
    if multi_thread {
        tokio::task::block_in_place(settle_key_readers);
    } else {
        settle_key_readers();
    }
}

pub(super) struct TerminalKey {
    cancel: Arc<AtomicBool>,
    answer: tokio::sync::oneshot::Receiver<AnswerKey>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl TerminalKey {
    pub(super) fn start(always: bool) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, answer) = tokio::sync::oneshot::channel();
        let flag = cancel.clone();
        let reader = std::thread::spawn(move || {
            // A reader of a withdrawn question may still be restoring the terminal.
            settle_key_readers();
            if let Some(key) = lineedit::read_answer(always, &flag) {
                let _ = send.send(key);
            }
        });
        Self { cancel, answer, reader: Some(reader) }
    }
}

impl Future for TerminalKey {
    type Output = AnswerKey;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<AnswerKey> {
        // A reader that stopped without a key (it cannot, unless cancelled) counts as no.
        Pin::new(&mut self.answer).poll(cx).map(|key| key.unwrap_or(AnswerKey::No))
    }
}

impl Drop for TerminalKey {
    /// Never blocks: the thread is told to stop and parked as pending (see [`terminal_keys`]).
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            if reader.is_finished() {
                let _ = reader.join();
            } else {
                PENDING_READERS.lock().unwrap_or_else(PoisonError::into_inner).push(reader);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_a_terminal_key_wait_does_not_block_on_its_reader() {
        // A reader that takes a while to notice the cancel (a slow poll): the drop returns at
        // once and the reader is settled by whoever touches the terminal next.
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let reader = std::thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(Duration::from_millis(300));
        });
        let (_send, answer) = tokio::sync::oneshot::channel();
        let wait = TerminalKey { cancel: cancel.clone(), answer, reader: Some(reader) };
        let started = Instant::now();
        drop(wait);
        assert!(started.elapsed() < Duration::from_millis(150), "the drop waited for the reader: {:?}", started.elapsed());
        assert!(cancel.load(Ordering::Acquire), "the reader is told to stop");
        settle_key_readers();
        assert!(started.elapsed() >= Duration::from_millis(300), "settling waits until the terminal is restored");
        assert!(PENDING_READERS.lock().unwrap().is_empty());
    }
}
