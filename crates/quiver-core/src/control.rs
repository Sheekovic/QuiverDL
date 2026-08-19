use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use tokio::sync::Notify;

use crate::{Error, Result};

const RUNNING: u8 = 0;
const PAUSED: u8 = 1;
const CANCELLED: u8 = 2;

/// A cheap, cloneable control handle for a running transfer.
#[derive(Debug, Clone)]
pub struct DownloadControl {
    inner: Arc<ControlInner>,
}

#[derive(Debug)]
struct ControlInner {
    state: AtomicU8,
    changed: Notify,
}

impl DownloadControl {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ControlInner {
                state: AtomicU8::new(RUNNING),
                changed: Notify::new(),
            }),
        }
    }

    pub fn pause(&self) {
        let _ =
            self.inner
                .state
                .compare_exchange(RUNNING, PAUSED, Ordering::AcqRel, Ordering::Acquire);
    }

    pub fn resume(&self) {
        if self.inner.state.swap(RUNNING, Ordering::AcqRel) == PAUSED {
            self.inner.changed.notify_waiters();
        }
    }

    pub fn cancel(&self) {
        self.inner.state.store(CANCELLED, Ordering::Release);
        self.inner.changed.notify_waiters();
    }

    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.inner.state.load(Ordering::Acquire) == PAUSED
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.state.load(Ordering::Acquire) == CANCELLED
    }

    /// Wait until the transfer is runnable, or return if it was cancelled.
    ///
    /// Desktop and service frontends can use this before handing a queued
    /// transfer to the engine so pause and cancellation semantics remain
    /// consistent while the transfer is waiting to start.
    pub async fn checkpoint(&self) -> Result<()> {
        loop {
            match self.inner.state.load(Ordering::Acquire) {
                RUNNING => return Ok(()),
                CANCELLED => return Err(Error::Cancelled),
                PAUSED => {
                    let changed = self.inner.changed.notified();
                    if self.inner.state.load(Ordering::Acquire) == PAUSED {
                        changed.await;
                    }
                }
                _ => unreachable!("invalid download control state"),
            }
        }
    }

    /// Wait until cancellation is requested.
    pub async fn cancelled(&self) {
        loop {
            let changed = self.inner.changed.notified();
            if self.is_cancelled() {
                return;
            }
            changed.await;
        }
    }
}

impl Default for DownloadControl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::DownloadControl;

    #[tokio::test]
    async fn paused_checkpoint_waits_for_resume() {
        let control = DownloadControl::new();
        control.pause();

        let waiter = tokio::spawn({
            let control = control.clone();
            async move { control.checkpoint().await }
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());

        control.resume();
        waiter
            .await
            .expect("task should join")
            .expect("resume works");
    }

    #[tokio::test]
    async fn cancellation_releases_a_paused_checkpoint() {
        let control = DownloadControl::new();
        control.pause();

        let waiter = tokio::spawn({
            let control = control.clone();
            async move { control.checkpoint().await }
        });

        control.cancel();
        assert!(waiter.await.expect("task should join").is_err());
    }
}
