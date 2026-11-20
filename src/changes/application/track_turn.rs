//! Snapshot and summarize around a turn without ever delaying it: both give up as soon as
//! the caller's cancel resolves.

use std::future::Future;
use std::path::PathBuf;

use crate::changes::domain::change_set::ChangeSummary;
use crate::changes::domain::snapshot::Snapshot;
use crate::changes::domain::turn_activity::TurnActivity;
use crate::changes::infrastructure::git_working_tree::{snapshot, summarize_turn};

/// Run blocking `work` on the blocking pool and give its result, or `None` as soon as
/// `cancel` resolves. The work is not stopped: it finishes in the background, and what
/// it does (git writing a temporary index, or reading) is harmless once nobody waits.
async fn until<T: Send + 'static>(cancel: impl Future<Output = ()>, work: impl FnOnce() -> Option<T> + Send + 'static) -> Option<T> {
    let task = tokio::task::spawn_blocking(work);
    tokio::select! {
        done = task => done.ok().flatten(),
        () = cancel => None,
    }
}

/// [`snapshot`] that returns `None` promptly once `cancel` resolves.
pub async fn snapshot_until(dir: PathBuf, cancel: impl Future<Output = ()>) -> Option<Snapshot> {
    until(cancel, move || snapshot(&dir)).await
}

/// [`summarize`] that returns `None` promptly once `cancel` resolves.
pub async fn summarize_until(
    dir: PathBuf,
    before: Snapshot,
    after: Snapshot,
    activity: TurnActivity,
    cancel: impl Future<Output = ()>,
) -> Option<ChangeSummary> {
    until(cancel, move || summarize_turn(&dir, &before, &after, &activity)).await
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::test_support::repo;

    #[tokio::test]
    async fn a_cancel_returns_at_once_while_a_slow_snapshot_finishes_in_the_background() {
        use std::time::{Duration, Instant};
        let started = Instant::now();
        let out = until(tokio::time::sleep(Duration::from_millis(20)), || {
            std::thread::sleep(Duration::from_millis(1000));
            Some("late")
        })
        .await;
        assert_eq!(out, None);
        assert!(started.elapsed() < Duration::from_millis(200), "took {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn without_a_cancel_the_blocking_result_comes_back() {
        assert_eq!(until(std::future::pending::<()>(), || Some(7)).await, Some(7));
        assert_eq!(until(std::future::pending::<()>(), || None::<u8>).await, None);
        let s = repo();
        let snap = snapshot_until(s.path().to_path_buf(), std::future::pending()).await.expect("a snapshot");
        let again = snapshot_until(s.path().to_path_buf(), std::future::pending()).await.unwrap();
        let sum = summarize_until(s.path().to_path_buf(), snap, again, TurnActivity::default(), std::future::pending()).await.unwrap();
        assert!(sum.is_empty());
    }
}
