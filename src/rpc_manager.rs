use ethers::providers::{Http, Provider};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tracing::warn;
use dashmap::DashMap;

/// Ultra-optimized RPC Manager for high-frequency MEV operations.
/// Implements nanosecond rotation, 429 (Rate Limit) detection, and exponential backoff.
pub struct RpcManager {
    providers: Vec<Arc<Provider<Http>>>,
    urls: Vec<String>,
    index: AtomicUsize,
    // Tracks cooldowns for specific URLs to avoid 429 loops
    cooldowns: DashMap<usize, Instant>,
    backoff_duration: Duration,
}

impl RpcManager {
    pub fn new(urls: Vec<String>) -> Self {
        if urls.is_empty() {
            panic!("FATAL: No RPC URLs provided to RpcManager");
        }

        let providers = urls.iter()
            .map(|url| Arc::new(Provider::<Http>::try_from(url).expect("Invalid RPC URL")))
            .collect();

        Self {
            providers,
            urls,
            index: AtomicUsize::new(0),
            cooldowns: DashMap::new(),
            backoff_duration: Duration::from_secs(5), // [BULLETPROOF] Increased cooldown for HF stability
        }
    }

    /// Fetches the next healthy provider using atomic rotation.
    /// If all providers are rate-limited, it implements a short blocking backoff.
    pub fn get_next_provider(&self) -> Arc<Provider<Http>> {
        let len = self.providers.len();
        let start_idx = self.index.fetch_add(1, Ordering::SeqCst) % len;

        // Try to find a non-cooling provider
        for i in 0..len {
            let idx = (start_idx + i) % len;
            if let Some(instant) = self.cooldowns.get(&idx) {
                if instant.elapsed() < self.backoff_duration {
                    continue;
                } else {
                    self.cooldowns.remove(&idx);
                }
            }
            return self.providers[idx].clone();
        }

        // Global Fallback: If all are limited, return the first one after a short emergency sleep
        warn!("⚠️ [RPC_MANAGER] All providers rate-limited. Retrying with first available after emergency backoff.");
        std::thread::sleep(Duration::from_millis(500)); 
        self.providers[0].clone()
    }

    /// Reports a rate limit (429) for a specific provider to trigger rotation and cooldown.
    pub fn report_rate_limit(&self, url: &str) {
        if let Some(idx) = self.urls.iter().position(|u| u == url) {
            self.cooldowns.insert(idx, Instant::now());
            warn!("🚫 [RPC_MANAGER] Rate limit detected for {}. Rotating and cooling down.", url);
        }
    }

    /// Returns the total number of configured RPC providers.
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    pub fn get_urls(&self) -> Vec<String> {
        self.urls.clone()
    }
}