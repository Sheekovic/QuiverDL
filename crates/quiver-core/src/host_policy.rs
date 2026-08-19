use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use tokio::sync::{Mutex, Notify};
use url::Url;

type HostStates = HashMap<String, Arc<HostState>>;

#[derive(Debug, Default)]
struct HostState {
    active: AtomicUsize,
    changed: Notify,
}

#[derive(Debug)]
pub(crate) struct HostPermit {
    state: Arc<HostState>,
}

impl Drop for HostPermit {
    fn drop(&mut self) {
        self.state.active.fetch_sub(1, Ordering::Release);
        self.state.changed.notify_one();
    }
}

/// Shared per-origin connection caps for cooperative download engines.
#[derive(Clone, Debug, Default)]
pub struct HostConnectionPolicy {
    hosts: Arc<Mutex<HostStates>>,
}

impl HostConnectionPolicy {
    pub(crate) async fn acquire(&self, url: &Url, max_connections: u8) -> Option<HostPermit> {
        let host = url.host_str()?.to_ascii_lowercase();
        let origin = format!(
            "{}://{}:{}",
            url.scheme(),
            host,
            url.port_or_known_default()?
        );
        let cap = usize::from(max_connections.clamp(1, 32));
        let state = {
            let mut hosts = self.hosts.lock().await;
            Arc::clone(hosts.entry(origin).or_default())
        };

        loop {
            let changed = state.changed.notified();
            let active = state.active.load(Ordering::Acquire);
            if active < cap {
                if state
                    .active
                    .compare_exchange(active, active + 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    drop(changed);
                    return Some(HostPermit { state });
                }
                continue;
            }
            changed.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;
    use url::Url;

    use super::HostConnectionPolicy;

    #[tokio::test]
    async fn enforces_and_releases_an_origin_cap() {
        let policy = HostConnectionPolicy::default();
        let url = Url::parse("https://example.test/file").expect("fixture URL");
        let first = policy.acquire(&url, 1).await.expect("first permit");
        assert!(
            timeout(Duration::from_millis(20), policy.acquire(&url, 1))
                .await
                .is_err()
        );
        drop(first);
        assert!(
            timeout(Duration::from_secs(1), policy.acquire(&url, 1))
                .await
                .expect("released permit should wake")
                .is_some()
        );
    }
}
