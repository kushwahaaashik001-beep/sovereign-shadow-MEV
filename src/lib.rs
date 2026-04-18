pub mod arbitrage_detector;
pub mod discovery;
pub mod bidding_engine;
pub mod bundle_builder;
pub mod constants;
pub mod errors;
pub mod factory_scanner;
pub mod flash_loan_executor;
pub mod gas_feed;
pub mod inventory_manager;
pub mod math_engine;
pub mod mempool_listener;
pub mod models;
pub mod nonce_manager;
pub mod profit_manager;
pub mod state_mirror;
pub mod state_simulator;
pub mod telemetry;
pub mod universal_decoder;
pub mod utils;
pub mod v3_math;

pub use utils::*;
pub use state_mirror::{StateMirror, GasState, PoolState};

use alloy::providers::RootProvider;
use alloy::transports::BoxTransport;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub type WsProvider = RootProvider<BoxTransport>;

pub struct WsProviderPool {
    pub providers: Vec<Arc<WsProvider>>,
    pub next: AtomicUsize,
    pub health: Vec<std::sync::atomic::AtomicU64>,
    pub usage_stats: Vec<std::sync::atomic::AtomicU64>, // Hydra Head Usage Tracking
}

impl WsProviderPool {
    pub fn new(providers: Vec<Arc<WsProvider>>) -> Self {
        let len = providers.len();
        let mut health = Vec::with_capacity(len);
        let mut usage = Vec::with_capacity(len);
        for _ in 0..len { 
            health.push(std::sync::atomic::AtomicU64::new(0)); 
            usage.push(std::sync::atomic::AtomicU64::new(0));
        }
        Self { providers, next: AtomicUsize::new(0), health, usage_stats: usage }
    }

    /// [HYDRA LOGIC] Returns (index, provider) to allow marking specific heads as unhealthy.
    pub fn next(&self) -> (usize, Arc<WsProvider>) {
        let len = self.providers.len();
        if len == 0 { panic!("No providers in pool"); }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        for _ in 0..len {
            let idx = self.next.fetch_add(1, Ordering::Relaxed) % len;
            // Check if this Hydra head is healthy (cooldown check)
            if self.health[idx].load(Ordering::Relaxed) <= now {
                self.usage_stats[idx].fetch_add(1, Ordering::Relaxed);
                return (idx, self.providers[idx].clone());
            }
        }

        // If all are blocked, return the least-blocked one
        (0, self.providers[0].clone())
    }

    pub fn mark_unhealthy(&self, provider_idx: usize, duration_secs: u64) {
        let resume_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + duration_secs;
        self.health[provider_idx].store(resume_at, Ordering::Relaxed);
    }
}
