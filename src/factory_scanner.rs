// The Sovereign Shadow - Z. The Zenith Protocol (Autonomous Factory Discovery)
// This module implements the FactoryScanner, which conforms to the architecture in main.rs.
// It listens for new pair creation events from major DEX factories on the configured chain
// and broadcasts them for other parts of the engine to consume.

use crate::constants;
use crate::models::DexName;
use ethers::prelude::*;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info, warn, debug};
use hex_literal::hex;
use rustc_hash::FxHashMap;

// Pillar Z: Pre-calculated Event Signatures for Nanosecond Dispatch
const V2_SIG: H256 = H256(hex!("0d3648bd0f6ba80134a332410a76efc102b4d6a0a031e3034a0e104e46046046"));
const V3_SIG: H256 = H256(hex!("783cca0653d2f9540b6e4e69ca578d3844f2d01135ed35272a0c64b58e709e9e"));
const AERO_SIG: H256 = H256(hex!("7c53369071450ce123365ad2faf951cc308a09bc0e596617bc7bb8bc4cc55ad2")); // PoolCreated for Aerodrome
const MAV_SIG: H256 = H256(hex!("2128d9a2b4ccfbc8a22222e6b8c9d10e599a099f66453272a0c64b58e709e9ee")); // PoolCreated for Maverick V2

/// Data for a Uniswap V2-style pool.
#[derive(Debug, Clone, Copy)]
pub struct V2PoolData {
    pub pair: Address,
    pub token_0: Address,
    pub token_1: Address,
    pub dex_name: DexName,
}

/// Data for a Uniswap V3-style pool.
#[derive(Debug, Clone, Copy)]
pub struct V3PoolData {
    pub pool: Address,
    pub token_0: Address,
    pub token_1: Address,
    pub fee: u32,
    pub dex_name: DexName,
}

/// Event that is broadcasted when a new liquidity pool is detected.
/// This is an enum to support different DEX protocols (e.g., V2 and V3 pools).
#[derive(Debug, Clone, Copy)]
pub enum NewPoolEvent {
    V2(V2PoolData),
    V3(V3PoolData),
}

/// The main struct for the factory scanner pillar.
pub struct FactoryScanner {
    ws_provider_pool: Arc<crate::WsProviderPool>, // Use the WsProviderPool
    pool_tx: broadcast::Sender<NewPoolEvent>,
    chain: Chain, // Pass chain from main.rs
}

impl FactoryScanner {
    /// Creates a new `FactoryScanner`.
    pub fn new(ws_provider_pool: Arc<crate::WsProviderPool>, pool_tx: broadcast::Sender<NewPoolEvent>, chain: Chain) -> Self {
        Self { ws_provider_pool, pool_tx, chain }
    }

    /// Runs the factory scanner task.
    pub async fn run(&self) {
        info!("🚀 [Pillar Z: Factory Scanner] Initializing...");

        info!("[Factory Scanner] Operating on chain: {:?} (ID: {})", self.chain, self.chain as u64);

        let mut factory_map: FxHashMap<Address, DexName> = FxHashMap::default();
        let mut factory_addresses: Vec<Address> = Vec::new();

        for ((c, dex), contracts) in constants::DEX_CONTRACTS.iter() {
            if *c == self.chain {
                info!("[Factory Scanner] Monitoring DEX: {:?} at factory {}", dex, contracts.factory);
                factory_map.insert(contracts.factory, *dex);
                factory_addresses.push(contracts.factory);
            }
        }

        if factory_addresses.is_empty() {
            warn!("[Factory Scanner] No factories found for chain {:?}. The scanner will be idle.", self.chain);
            return;
        }

        loop {
            let provider = self.ws_provider_pool.next();

            // Pillar Z: Universal Discovery Filter
            let filter = Filter::new().topic0(vec![V2_SIG, V3_SIG, AERO_SIG, MAV_SIG]);

            let mut stream: SubscriptionStream<'_, Ws, Log> = match provider.subscribe_logs(&filter).await {
                Ok(s) => s,
                Err(e) => {
                    error!("[Factory Scanner] WSS Subscription failed: {}. Retrying with next provider.", e);
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };

            info!("[Factory Scanner] Listening for new pool creation events...");

            while let Some(log) = stream.next().await {
                let dex_name = factory_map.get(&log.address).cloned().unwrap_or_else(|| {
                    if log.topics[0] == V2_SIG { DexName::UniswapV2 } 
                    else if log.topics[0] == V3_SIG { DexName::UniswapV3 }
                    else { DexName::UniswapV2 }
                });

                let event_sig = log.topics[0];
                
                let event = if event_sig == V2_SIG {
                    if log.topics.len() >= 3 && log.data.len() >= 64 {
                        let token_0 = Address::from_slice(&log.topics[1][12..32]);
                        let token_1 = Address::from_slice(&log.topics[2][12..32]);
                        let pair = Address::from_slice(&log.data[12..32]);
                        info!("✨ [ZENITH] Custom DEX Detected (V2)! Pair: {:?} | T0: {:?} T1: {:?}", pair, token_0, token_1);
                        Some(NewPoolEvent::V2(V2PoolData { pair, token_0, token_1, dex_name: dex_name.clone() }))
                    } else { None }
                } else if event_sig == V3_SIG {
                    if log.topics.len() >= 4 && log.data.len() >= 64 {
                        let token_0 = Address::from_slice(&log.topics[1][12..32]);
                        let token_1 = Address::from_slice(&log.topics[2][12..32]);
                        let fee = u32::from_be_bytes([0, log.topics[3][29], log.topics[3][30], log.topics[3][31]]);
                        let pool = Address::from_slice(&log.data[12..32]);
                        info!("✨ [ZENITH] Custom DEX Detected (V3)! Pool: {:?} | Fee: {}", pool, fee);
                        Some(NewPoolEvent::V3(V3PoolData { pool, token_0, token_1, fee, dex_name: dex_name.clone() }))
                    } else { None }
                } else if event_sig == AERO_SIG {
                    if log.topics.len() >= 3 && log.data.len() >= 64 {
                        let token_0 = Address::from_slice(&log.topics[1][12..32]);
                        let token_1 = Address::from_slice(&log.topics[2][12..32]);
                        let pool = Address::from_slice(&log.data[44..64]); 
                        debug!("🆕 [AERO] Pool: {:?} | T0: {:?} T1: {:?}", pool, token_0, token_1);
                        Some(NewPoolEvent::V2(V2PoolData { pair: pool, token_0, token_1, dex_name: dex_name.clone() }))
                    } else { None }
                } else if event_sig == MAV_SIG {
                    if log.data.len() >= 128 {
                        let pool = Address::from_slice(&log.data[12..32]);
                        let token_0 = Address::from_slice(&log.data[44..64]);
                        let token_1 = Address::from_slice(&log.data[76..96]);
                        let fee = u32::from_be_bytes([log.data[124], log.data[125], log.data[126], log.data[127]]);
                        debug!("🆕 [MAV] Pool: {:?} | T0: {:?} T1: {:?} Fee: {}", pool, token_0, token_1, fee);
                        Some(NewPoolEvent::V3(V3PoolData { pool, token_0, token_1, fee, dex_name: dex_name.clone() }))
                    } else { None }
                } else {
                    None
                };

                if let Some(event_to_send) = event {
                    if self.pool_tx.send(event_to_send).is_err() {
                        warn!("[Factory Scanner] No active listeners for pool events.");
                    }
                }
            }
            error!("[Factory Scanner] WSS stream ended unexpectedly. Reconnecting with next provider...");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
}
