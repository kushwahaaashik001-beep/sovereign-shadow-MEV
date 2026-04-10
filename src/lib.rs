#[macro_use]
pub mod constants;
pub mod arbitrage_detector;
pub mod auditor;
pub mod bidding_engine;
pub mod bindings;
pub mod bundle_builder;
pub mod discovery;
pub mod errors;
pub mod factory_scanner;
pub mod flash_loan_executor;
pub mod gas_feed;
pub mod ghost_protocol;
pub mod inventory_manager;
pub mod math_engine;
pub mod mempool_listener;
pub mod models;
pub mod multicall;
pub mod nonce_manager;
pub mod rpc_manager;
pub mod profit_manager;
pub mod state_mirror;
pub mod state_simulator;
pub mod universal_decoder;
pub mod universal_decoder_helpers;
pub mod utils;
pub mod v3_math;

pub use models::*;
pub use utils::*;
pub use state_mirror::{StateMirror, GasState, PoolState};

use ethers::providers::{Provider, Ws};
use std::sync::Arc;

pub struct WsProviderPool {
    pub providers: Vec<Arc<Provider<Ws>>>,
    pub next:      std::sync::atomic::AtomicUsize,
}

impl WsProviderPool {
    pub fn new(providers: Vec<Arc<Provider<Ws>>>) -> Self {
        Self { providers, next: std::sync::atomic::AtomicUsize::new(0) }
    }
    pub fn next(&self) -> Arc<Provider<Ws>> {
        let idx = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % self.providers.len();
        self.providers[idx].clone()
    }
}
