use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::{sync::Notify, time::Instant};

#[derive(Debug)]
struct Schedule {
    next_available: Instant,
}

/// A cloneable bandwidth scheduler that can be shared by several downloads.
#[derive(Clone, Debug)]
pub struct BandwidthLimiter {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    bytes_per_second: AtomicU64,
    schedule: Mutex<Schedule>,
    changed: Notify,
}

impl BandwidthLimiter {
    #[must_use]
    pub fn new(bytes_per_second: u64) -> Option<Self> {
        (bytes_per_second > 0).then(|| Self {
            inner: Arc::new(Inner {
                bytes_per_second: AtomicU64::new(bytes_per_second),
                schedule: Mutex::new(Schedule {
                    next_available: Instant::now(),
                }),
                changed: Notify::new(),
            }),
        })
    }

    #[must_use]
    pub fn unlimited() -> Self {
        Self {
            inner: Arc::new(Inner {
                bytes_per_second: AtomicU64::new(0),
                schedule: Mutex::new(Schedule {
                    next_available: Instant::now(),
                }),
                changed: Notify::new(),
            }),
        }
    }

    pub fn set_bytes_per_second(&self, bytes_per_second: u64) {
        self.inner
            .bytes_per_second
            .store(bytes_per_second, Ordering::Release);
        self.inner
            .schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .next_available = Instant::now();
        self.inner.changed.notify_waiters();
    }

    pub(crate) async fn wait(&self, bytes: usize) {
        if bytes == 0 {
            return;
        }
        loop {
            let changed = self.inner.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let bytes_per_second = self.inner.bytes_per_second.load(Ordering::Acquire);
            if bytes_per_second == 0 {
                return;
            }
            let now = Instant::now();
            let start = {
                let mut schedule = self
                    .inner
                    .schedule
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let start = schedule.next_available.max(now);
                let seconds = bytes as f64 / bytes_per_second as f64;
                schedule.next_available = start + Duration::from_secs_f64(seconds);
                start
            };
            if start <= now {
                return;
            }
            tokio::select! {
                () = tokio::time::sleep_until(start) => return,
                () = &mut changed => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;

    use super::BandwidthLimiter;

    #[tokio::test]
    async fn disabling_a_shared_limit_wakes_existing_waiters() {
        let limiter = BandwidthLimiter::new(1).expect("positive limit");
        limiter.wait(1).await;
        let waiting = tokio::spawn({
            let limiter = limiter.clone();
            async move { limiter.wait(1).await }
        });
        tokio::task::yield_now().await;
        limiter.set_bytes_per_second(0);
        timeout(Duration::from_millis(100), waiting)
            .await
            .expect("disabled limiter should wake")
            .expect("waiter should join");
    }
}
