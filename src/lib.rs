pub mod arbitrage_detector;
pub mod discovery;
pub mod rpc_manager;
pub mod bidding_engine;
pub mod bindings;
pub mod bundle_builder;
pub mod constants;
pub mod errors;
pub mod factory_scanner;
pub mod flash_loan_executor;
pub mod gas_feed;
pub mod inventory_manager;
pub mod p2p_engine;
pub mod math_engine;
pub mod mempool_listener;
pub mod models;
pub mod multicall;
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
}

impl WsProviderPool {
    pub fn new(providers: Vec<Arc<WsProvider>>) -> Self {
        Self { providers, next: AtomicUsize::new(0) }
    }
    pub fn next(&self) -> Arc<WsProvider> {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.providers.len().max(1);
        self.providers[idx].clone()
    }
}
