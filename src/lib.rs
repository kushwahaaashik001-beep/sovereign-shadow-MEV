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
    pub health: Vec<std::sync::atomic::AtomicBool>,
}

impl WsProviderPool {
    pub fn new(providers: Vec<Arc<WsProvider>>) -> Self {
        let len = providers.len();
        let mut health = Vec::with_capacity(len);
        for _ in 0..len { health.push(std::sync::atomic::AtomicBool::new(true)); }
        Self { providers, next: AtomicUsize::new(0), health }
    }

    /// THE LOAD BALANCER: Har call par agla healthy provider return karta hai.
    pub fn next(&self) -> Arc<WsProvider> {
        let start_idx = self.next.fetch_add(1, Ordering::Relaxed) % self.providers.len().max(1);
        
        // Try to find the next healthy provider starting from current index
        for i in 0..self.providers.len() {
            let idx = (start_idx + i) % self.providers.len();
            if self.health[idx].load(Ordering::Relaxed) {
                return self.providers[idx].clone();
            }
        }
        
        // Fallback to the first one if all are "unhealthy"
        self.providers[0].clone()
    }

    pub fn mark_unhealthy(&self, provider_idx: usize) {
        self.health[provider_idx].store(false, Ordering::Relaxed);
    }
}
