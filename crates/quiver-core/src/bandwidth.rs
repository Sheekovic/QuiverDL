use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::{sync::Mutex, time::Instant};

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
            }),
        })
    }

    pub fn set_bytes_per_second(&self, bytes_per_second: u64) {
        self.inner
            .bytes_per_second
            .store(bytes_per_second.max(1), Ordering::Relaxed);
    }

    pub(crate) async fn wait(&self, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let now = Instant::now();
        let mut schedule = self.inner.schedule.lock().await;
        let start = schedule.next_available.max(now);
        let bytes_per_second = self.inner.bytes_per_second.load(Ordering::Relaxed).max(1);
        let seconds = bytes as f64 / bytes_per_second as f64;
        schedule.next_available = start + Duration::from_secs_f64(seconds);
        drop(schedule);
        if start > now {
            tokio::time::sleep_until(start).await;
        }
    }
}
